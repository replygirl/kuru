//! Ordered admission for native reads that can execute independently.
//!
//! A prepared read is not a permission grant. Every effect rechecks the exact
//! authority captured here, and foreground/Once decisions use the ordinary
//! immediate-dispatch path instead of becoming transferable tokens.

use std::{
    ffi::OsString,
    fs::File,
    io::Read,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Context, Result};
use grep_regex::RegexMatcher;
use kuru_core::{NativeTool, PermissionSelector, ProjectRelativeTarget};
use kuru_platform::fs::{Directory, NameRetention, Privacy, regular_file_info};
use serde_json::{Value, json};
use tokio::sync::Notify;

use super::{
    ActorToolOutcome, InstructionGateOutcome, MAX_SEARCH_MATCHES, PermissionInvocation,
    SearchCandidateAccess, SearchOmissions, ToolExecution, ToolFailure, ToolHost,
    ToolInvocationContext, ToolInvocationOrigin, bounded_search_pattern, instruction_directory,
    project_execution, project_failure, string,
};

pub(super) const MAX_PREPARED_READ_HANDLES: usize = 96;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReadKind {
    FileRead,
    FileList,
    Grep,
    Glob,
    WebFetch,
}

impl ReadKind {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "file_read" => Some(Self::FileRead),
            "file_list" => Some(Self::FileList),
            "grep" => Some(Self::Grep),
            "glob" => Some(Self::Glob),
            "web_fetch" => Some(Self::WebFetch),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::FileRead => "file_read",
            Self::FileList => "file_list",
            Self::Grep => "grep",
            Self::Glob => "glob",
            Self::WebFetch => "web_fetch",
        }
    }

    fn search_tool(self) -> Option<NativeTool> {
        match self {
            Self::Grep => Some(NativeTool::Grep),
            Self::Glob => Some(NativeTool::Glob),
            _ => None,
        }
    }
}

/// A call that cannot be safely held across ordered admission remains on the
/// existing serial ToolHost path, including fresh foreground/Once decisions.
pub enum ParallelReadAdmission {
    Ready(Box<PreparedRead>),
    Serial,
}

/// Wave-local cancellation used after runtime admission. Blocking filesystem
/// work still drains its owned worker; async fetch/read work stops promptly.
#[derive(Clone, Default)]
pub struct ParallelReadCancellation {
    inner: Arc<ParallelReadCancellationInner>,
}

#[derive(Default)]
struct ParallelReadCancellationInner {
    cancelled: AtomicBool,
    changed: Notify,
}

impl ParallelReadCancellation {
    pub fn cancel(&self) {
        if !self.inner.cancelled.swap(true, Ordering::AcqRel) {
            self.inner.changed.notify_waiters();
        }
    }

    fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }

    async fn cancelled(&self) {
        loop {
            let changed = self.inner.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            changed.await;
        }
    }
}

/// Exact native read facts captured before a parallel execution wave.
pub struct PreparedRead {
    kind: ReadKind,
    args: Value,
    context: ToolInvocationContext,
    target: Option<ProjectRelativeTarget>,
    captured_target: Option<CapturedTarget>,
    candidates: Vec<ProjectRelativeTarget>,
    captured_candidates: Vec<CapturedFile>,
    omissions: SearchOmissions,
}

struct CapturedFile {
    target: ProjectRelativeTarget,
    parent: Directory,
    name: OsString,
    file: File,
    _handles: RetainedHandlePermit,
}

struct CapturedDirectory {
    target: ProjectRelativeTarget,
    directory: Directory,
    _handles: RetainedHandlePermit,
}

struct RetainedHandlePermit {
    usage: Arc<std::sync::atomic::AtomicUsize>,
    count: usize,
}

impl RetainedHandlePermit {
    fn acquire(usage: Arc<std::sync::atomic::AtomicUsize>, count: usize) -> Option<Self> {
        usage
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current
                    .checked_add(count)
                    .filter(|total| *total <= MAX_PREPARED_READ_HANDLES)
            })
            .ok()?;
        Some(Self { usage, count })
    }
}

impl Drop for RetainedHandlePermit {
    fn drop(&mut self) {
        let previous = self.usage.fetch_sub(self.count, Ordering::AcqRel);
        debug_assert!(previous >= self.count);
    }
}

enum CapturedTarget {
    File(CapturedFile),
    Directory(CapturedDirectory),
}

impl CapturedFile {
    fn verify(&self, root: &Directory) -> Result<()> {
        root.revalidate()?;
        anyhow::ensure!(
            self.parent.is_within(root)?,
            "prepared file parent left the retained project root"
        );
        self.parent.verify(&self.name, &self.file)?;
        Ok(())
    }

    fn checked_clone(&self, root: &Directory) -> Result<File> {
        self.verify(root)?;
        let file = self.file.try_clone()?;
        anyhow::ensure!(
            regular_file_info(&file)?.links == 1,
            "prepared file gained another hardlink"
        );
        Ok(file)
    }
}

impl CapturedDirectory {
    fn verify(&self, root: &Directory) -> Result<()> {
        root.revalidate()?;
        anyhow::ensure!(
            self.directory.is_within(root)?,
            "prepared directory left the retained project root"
        );
        Ok(())
    }
}

/// Test-only two-phase gate for observing checked reads before allowing their
/// effects to continue. Production admission contains no shared wave gate.
#[cfg(any(test, feature = "test-support"))]
#[derive(Clone)]
pub struct ParallelReadTestGate {
    inner: Arc<ParallelReadTestGateInner>,
}

#[cfg(any(test, feature = "test-support"))]
struct ParallelReadTestGateInner {
    expected: usize,
    entered: std::sync::atomic::AtomicUsize,
    entered_changed: Notify,
    release_all: AtomicBool,
    released: std::sync::Mutex<std::collections::BTreeSet<String>>,
    contexts: std::sync::Mutex<Vec<(String, ToolInvocationContext)>>,
    release_changed: Notify,
}

#[cfg(any(test, feature = "test-support"))]
impl ParallelReadTestGate {
    pub fn new(expected: usize) -> Self {
        assert!(expected > 0, "parallel test gate needs an expected caller");
        Self {
            inner: Arc::new(ParallelReadTestGateInner {
                expected,
                entered: std::sync::atomic::AtomicUsize::new(0),
                entered_changed: Notify::new(),
                release_all: AtomicBool::new(false),
                released: std::sync::Mutex::new(std::collections::BTreeSet::new()),
                contexts: std::sync::Mutex::new(Vec::new()),
                release_changed: Notify::new(),
            }),
        }
    }

    pub async fn wait_until_entered(&self) {
        loop {
            let changed = self.inner.entered_changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.inner.entered.load(Ordering::Acquire) >= self.inner.expected {
                return;
            }
            changed.await;
        }
    }

    pub fn release(&self) {
        self.inner.release_all.store(true, Ordering::Release);
        self.inner.release_changed.notify_waiters();
    }

    /// Release callers for one exact prepared target or read kind.
    pub fn release_named(&self, name: &str) {
        self.inner
            .released
            .lock()
            .expect("parallel gate release set is poisoned")
            .insert(name.to_owned());
        self.inner.release_changed.notify_waiters();
    }

    /// Exact actor provenance observed at the checked execution boundary.
    pub fn entered_contexts(&self) -> Vec<(String, ToolInvocationContext)> {
        self.inner
            .contexts
            .lock()
            .expect("parallel gate context set is poisoned")
            .clone()
    }

    async fn enter(&self, name: &str, context: &ToolInvocationContext) {
        let entry = self.inner.entered.fetch_add(1, Ordering::AcqRel);
        assert!(
            entry < self.inner.expected,
            "too many parallel gate callers"
        );
        self.inner
            .contexts
            .lock()
            .expect("parallel gate context set is poisoned")
            .push((name.to_owned(), context.clone()));
        self.inner.entered_changed.notify_waiters();
        loop {
            let changed = self.inner.release_changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.inner.release_all.load(Ordering::Acquire)
                || self
                    .inner
                    .released
                    .lock()
                    .expect("parallel gate release set is poisoned")
                    .contains(name)
            {
                return;
            }
            changed.await;
        }
    }
}

impl ToolHost {
    /// Perform only read-only, non-publishing admission. The caller must use
    /// ordinary dispatch for `Serial` before admitting the next provider call.
    pub async fn prepare_parallel_read(
        &self,
        name: &str,
        args: Value,
        context: ToolInvocationContext,
    ) -> ParallelReadAdmission {
        let Some(kind) = ReadKind::from_name(name) else {
            return ParallelReadAdmission::Serial;
        };
        if !args.is_object() {
            return ParallelReadAdmission::Serial;
        }
        let mut prepared = PreparedRead {
            kind,
            args,
            context,
            target: None,
            captured_target: None,
            candidates: Vec::new(),
            captured_candidates: Vec::new(),
            omissions: SearchOmissions::default(),
        };
        if let Some(tool) = kind.search_tool() {
            if !self
                .permissions
                .advertises(&PermissionSelector::native(tool))
            {
                return ParallelReadAdmission::Serial;
            }
            let pattern = match kind {
                ReadKind::Glob => {
                    match string(&prepared.args, "pattern").and_then(bounded_search_pattern) {
                        Ok(pattern) => Some(pattern),
                        Err(_) => return ParallelReadAdmission::Serial,
                    }
                }
                ReadKind::Grep => {
                    let Ok(pattern) =
                        string(&prepared.args, "pattern").and_then(bounded_search_pattern)
                    else {
                        return ParallelReadAdmission::Serial;
                    };
                    if RegexMatcher::new_line_matcher(&pattern).is_err() {
                        return ParallelReadAdmission::Serial;
                    }
                    None
                }
                _ => unreachable!(),
            };
            let Ok(candidates) =
                self.search_candidates(&prepared.args, pattern.as_deref(), &mut prepared.omissions)
            else {
                return ParallelReadAdmission::Serial;
            };
            for candidate in candidates {
                match self
                    .authorize_search_candidate(tool, &candidate, &prepared.args, None)
                    .await
                {
                    Ok(SearchCandidateAccess::Allowed) => prepared.candidates.push(candidate),
                    Ok(SearchCandidateAccess::Denied) => prepared.omissions.denied += 1,
                    Ok(SearchCandidateAccess::PermissionRequired) | Err(_) => {
                        return ParallelReadAdmission::Serial;
                    }
                }
                if tool == NativeTool::Glob && prepared.candidates.len() == MAX_SEARCH_MATCHES {
                    prepared.omissions.output_limit = true;
                    break;
                }
            }
            if self
                .parallel_review(&prepared.candidates, false)
                .await
                .is_err()
            {
                return ParallelReadAdmission::Serial;
            }
            let Ok(captured) = prepared
                .candidates
                .iter()
                .map(|target| self.capture_file(target))
                .collect::<Result<Vec<_>>>()
            else {
                return ParallelReadAdmission::Serial;
            };
            prepared.captured_candidates = captured;
        } else {
            let Ok((selector, target)) = self.permission_facts(name, &prepared.args).await else {
                return ParallelReadAdmission::Serial;
            };
            let Ok(invocation) =
                PermissionInvocation::new(selector, target.clone(), &prepared.args)
            else {
                return ParallelReadAdmission::Serial;
            };
            if !self
                .permissions
                .authorize(&invocation, None)
                .await
                .is_ok_and(|decision| decision.is_authorized())
            {
                return ParallelReadAdmission::Serial;
            }
            prepared.target = target;
            if let Some(target) = prepared.target.as_ref()
                && self
                    .parallel_review(std::slice::from_ref(target), kind == ReadKind::FileList)
                    .await
                    .is_err()
            {
                return ParallelReadAdmission::Serial;
            }
            prepared.captured_target = match (kind, prepared.target.as_ref()) {
                (ReadKind::FileRead, Some(target)) => match self.capture_file(target) {
                    Ok(file) => Some(CapturedTarget::File(file)),
                    Err(_) => return ParallelReadAdmission::Serial,
                },
                (ReadKind::FileList, Some(target)) => match self.capture_directory(target) {
                    Ok(directory) => Some(CapturedTarget::Directory(directory)),
                    Err(_) => return ParallelReadAdmission::Serial,
                },
                _ => None,
            };
        }
        ParallelReadAdmission::Ready(Box::new(prepared))
    }

    fn capture_file(&self, target: &ProjectRelativeTarget) -> Result<CapturedFile> {
        self.root_guard.revalidate()?;
        let relative = Path::new(target.as_str());
        let name = relative
            .file_name()
            .context("prepared file target lacks a final component")?
            .to_os_string();
        let parent_path = self
            .root
            .join(relative.parent().unwrap_or_else(|| Path::new(".")));
        let handles = RetainedHandlePermit::acquire(
            self.parallel_read_handles.clone(),
            parent_path.ancestors().count() + 1,
        )
        .context("prepared read retained-handle budget is exhausted")?;
        let parent = Directory::open(&parent_path, Privacy::Inherited, NameRetention::Pinned)?;
        anyhow::ensure!(
            parent.is_within(&self.root_guard)?,
            "prepared file parent is outside the retained project root"
        );
        let file = parent.read(&name)?;
        Ok(CapturedFile {
            target: target.clone(),
            parent,
            name,
            file,
            _handles: handles,
        })
    }

    fn capture_directory(&self, target: &ProjectRelativeTarget) -> Result<CapturedDirectory> {
        self.root_guard.revalidate()?;
        let path = self.root.join(target.as_str());
        let handles = RetainedHandlePermit::acquire(
            self.parallel_read_handles.clone(),
            path.ancestors().count(),
        )
        .context("prepared read retained-handle budget is exhausted")?;
        let directory = Directory::open(&path, Privacy::Inherited, NameRetention::Pinned)?;
        anyhow::ensure!(
            directory.is_within(&self.root_guard)?,
            "prepared directory is outside the retained project root"
        );
        Ok(CapturedDirectory {
            target: target.clone(),
            directory,
            _handles: handles,
        })
    }

    async fn parallel_review(
        &self,
        targets: &[ProjectRelativeTarget],
        directory_target: bool,
    ) -> Result<()> {
        let Some(gate) = &self.instruction_gate else {
            return Ok(());
        };
        if targets.is_empty() {
            return Ok(());
        }
        let directories = targets
            .iter()
            .map(|target| instruction_directory(target, directory_target))
            .collect::<Result<Vec<_>>>()?;
        match gate.review(&directories, None).await? {
            InstructionGateOutcome::Unchanged => Ok(()),
            // Proposed activation and a missing foreground reviewer are both
            // serial boundaries; this phase never publishes prompt authority.
            _ => anyhow::bail!("instruction review requires serial dispatch"),
        }
    }
}

impl PreparedRead {
    /// Recheck authority immediately before this admitted read's effect. The
    /// caller owns and drains the returned future, including blocking workers.
    pub async fn execute(
        self: Box<Self>,
        host: Arc<ToolHost>,
        cancellation: ParallelReadCancellation,
    ) -> ActorToolOutcome {
        let result = self.execute_checked(host, cancellation).await;
        ActorToolOutcome {
            result: match result {
                Ok(execution) => project_execution(execution),
                Err(failure) => project_failure(failure),
            },
            instructions: None,
            replan_required: false,
        }
    }

    async fn execute_checked(
        self: Box<Self>,
        host: Arc<ToolHost>,
        cancellation: ParallelReadCancellation,
    ) -> std::result::Result<ToolExecution, ToolFailure> {
        host.root_guard
            .revalidate()
            .map_err(|error| ToolFailure::built_in(error.into()))?;
        if cancellation.is_cancelled() {
            return Err(cancelled_read());
        }
        #[cfg(any(test, feature = "test-support"))]
        if let Some(barrier) = &host.parallel_read_barrier {
            barrier.wait().await;
        }
        if let Some(tool) = self.kind.search_tool() {
            if self.candidates.len() != self.captured_candidates.len() {
                return Err(changed_read_authority());
            }
            for (target, captured) in self.candidates.iter().zip(&self.captured_candidates) {
                if captured.target != *target || captured.verify(&host.root_guard).is_err() {
                    return Err(changed_read_authority());
                }
                let current = host
                    .validated_permission_target(target.as_str(), false)
                    .map_err(ToolFailure::built_in)?;
                if current != *target {
                    return Err(changed_read_authority());
                }
                match host
                    .authorize_search_candidate(tool, target, &self.args, None)
                    .await
                    .map_err(ToolFailure::built_in)?
                {
                    SearchCandidateAccess::Allowed => {}
                    _ => return Err(changed_read_authority()),
                }
            }
            host.parallel_review(&self.candidates, false)
                .await
                .map_err(|_| changed_read_authority())?;
        } else {
            let (selector, current_target) =
                host.permission_facts(self.kind.name(), &self.args).await?;
            if current_target != self.target {
                return Err(changed_read_authority());
            }
            let invocation = PermissionInvocation::new(selector, current_target, &self.args)
                .map_err(ToolFailure::built_in)?;
            if !host
                .permissions
                .authorize(&invocation, None)
                .await
                .map_err(ToolFailure::built_in)?
                .is_authorized()
            {
                return Err(changed_read_authority());
            }
            if let Some(target) = self.target.as_ref() {
                host.parallel_review(
                    std::slice::from_ref(target),
                    self.kind == ReadKind::FileList,
                )
                .await
                .map_err(|_| changed_read_authority())?;
            }
            match &self.captured_target {
                Some(CapturedTarget::File(captured))
                    if Some(&captured.target) == self.target.as_ref()
                        && captured.verify(&host.root_guard).is_ok() => {}
                Some(CapturedTarget::Directory(captured))
                    if Some(&captured.target) == self.target.as_ref()
                        && captured.verify(&host.root_guard).is_ok() => {}
                None if matches!(self.kind, ReadKind::WebFetch) => {}
                _ => return Err(changed_read_authority()),
            }
        }

        if self.kind == ReadKind::Glob {
            #[cfg(any(test, feature = "test-support"))]
            if let Some(gate) = &host.parallel_read_test_gate {
                gate.enter(self.kind.name(), &self.context).await;
            }
            return Ok(ToolExecution::Json(json!({
                "matches": self.candidates.iter().map(ProjectRelativeTarget::as_str).collect::<Vec<_>>(),
                "omitted": self.omissions.into_json(),
            })));
        }
        if self.kind == ReadKind::WebFetch {
            #[cfg(any(test, feature = "test-support"))]
            if let Some(gate) = &host.parallel_read_test_gate {
                gate.enter(self.kind.name(), &self.context).await;
            }
            let mut instructions = None;
            return tokio::select! {
                biased;
                () = cancellation.cancelled() => Err(cancelled_read()),
                result = host.execute_inner(
                        self.kind.name(),
                        self.args,
                        None,
                        None,
                        false,
                        ToolInvocationOrigin::Actor(&self.context),
                        None,
                        &mut instructions,
                    ) => result,
            };
        }
        // Checked filesystem work contains synchronous directory and search
        // operations. Run it off the async executor, bounded by the caller's
        // wave limit, while retaining this task's JoinHandle through cleanup.
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            runtime.block_on(async move {
                #[cfg(any(test, feature = "test-support"))]
                if let Some(gate) = &host.parallel_read_test_gate {
                    gate.enter(
                        self.target
                            .as_ref()
                            .map_or_else(|| self.kind.name(), ProjectRelativeTarget::as_str),
                        &self.context,
                    )
                    .await;
                }
                if cancellation.is_cancelled() {
                    return Err(cancelled_read());
                }
                if self.kind == ReadKind::Grep {
                    let pattern = bounded_search_pattern(
                        string(&self.args, "pattern").map_err(ToolFailure::built_in)?,
                    )
                    .map_err(ToolFailure::built_in)?;
                    let matcher = RegexMatcher::new_line_matcher(&pattern)
                        .context("grep pattern is not a valid line regular expression")
                        .map_err(ToolFailure::built_in)?;
                    return grep_captured_targets(
                        &host.root_guard,
                        &matcher,
                        self.captured_candidates,
                        self.omissions,
                    )
                    .map(ToolExecution::Json);
                }
                if self.kind == ReadKind::FileRead {
                    let Some(CapturedTarget::File(captured)) = self.captured_target else {
                        return Err(changed_read_authority());
                    };
                    return read_captured_file(&host.root_guard, &self.args, &captured).await;
                }
                let mut instructions = None;
                let result = host
                    .execute_inner(
                        self.kind.name(),
                        self.args,
                        None,
                        None,
                        false,
                        ToolInvocationOrigin::Actor(&self.context),
                        self.target.as_ref(),
                        &mut instructions,
                    )
                    .await;
                if let Some(CapturedTarget::Directory(captured)) = &self.captured_target
                    && captured.verify(&host.root_guard).is_err()
                {
                    return Err(changed_read_authority());
                }
                result
            })
        })
        .await
        .map_err(|error| ToolFailure::built_in(error.into()))?
    }
}

async fn read_captured_file(
    root: &Directory,
    args: &Value,
    captured: &CapturedFile,
) -> std::result::Result<ToolExecution, ToolFailure> {
    let file = captured
        .checked_clone(root)
        .map_err(|_| changed_read_authority())?;
    let extent = regular_file_info(&file)
        .map_err(|error| ToolFailure::built_in(error.into()))?
        .len;
    let result = if args.get("offset").is_some() || args.get("limit").is_some() {
        ToolExecution::Json(
            super::page_file_read(
                file,
                extent,
                super::page_offset(args).map_err(ToolFailure::built_in)?,
                super::page_limit(args).map_err(ToolFailure::built_in)?,
            )
            .map_err(ToolFailure::built_in)?,
        )
    } else {
        ToolExecution::ProjectedText(
            super::read_file_output(tokio::fs::File::from_std(file), extent)
                .await
                .map_err(ToolFailure::built_in)?,
        )
    };
    captured
        .verify(root)
        .map_err(|_| changed_read_authority())?;
    Ok(result)
}

fn grep_captured_targets(
    root: &Directory,
    matcher: &RegexMatcher,
    targets: Vec<CapturedFile>,
    mut omissions: SearchOmissions,
) -> std::result::Result<Value, ToolFailure> {
    let mut matches = Vec::new();
    for target in targets {
        let mut file = target
            .checked_clone(root)
            .map_err(|_| changed_read_authority())?;
        let metadata =
            regular_file_info(&file).map_err(|error| ToolFailure::built_in(error.into()))?;
        if metadata.len > super::MAX_SEARCH_FILE_BYTES {
            omissions.large_or_non_file += 1;
            continue;
        }
        let mut bytes = Vec::with_capacity(usize::try_from(metadata.len).unwrap_or(0));
        Read::by_ref(&mut file)
            .take(super::MAX_SEARCH_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| ToolFailure::built_in(error.into()))?;
        target.verify(root).map_err(|_| changed_read_authority())?;
        if bytes.len() as u64 > super::MAX_SEARCH_FILE_BYTES {
            omissions.large_or_non_file += 1;
            continue;
        }
        if std::str::from_utf8(&bytes).is_err() {
            omissions.unreadable += 1;
            continue;
        }
        let target_name = target.target.as_str().to_owned();
        let mut searcher = grep_searcher::Searcher::new();
        searcher
            .search_slice(
                matcher,
                &bytes,
                grep_searcher::sinks::UTF8(|line, text| {
                    if text.len() > super::MAX_SEARCH_LINE_BYTES {
                        omissions.oversized_line += 1;
                        return Ok(true);
                    }
                    matches.push(json!({"path": target_name, "line": line, "text": text.trim_end_matches(['\n', '\r'])}));
                    Ok(matches.len() < super::MAX_SEARCH_MATCHES)
                }),
            )
            .map_err(|error| ToolFailure::built_in(anyhow::anyhow!("grep search failed: {error}")))?;
        if matches.len() == super::MAX_SEARCH_MATCHES {
            omissions.output_limit = true;
            break;
        }
    }
    Ok(json!({"matches": matches, "omitted": omissions.into_json()}))
}

fn changed_read_authority() -> ToolFailure {
    ToolFailure::permission_denied(anyhow::anyhow!(
        "read authority changed after ordered admission; replan this call"
    ))
}

fn cancelled_read() -> ToolFailure {
    ToolFailure::built_in(anyhow::anyhow!("parallel read cancelled"))
}
