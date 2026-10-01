//! The plain sentence the command line shows while project memory opens.
//!
//! Every text a user can see here is one constant in this module. Memory
//! reports coarse open stages; this module maps them to one of five
//! sentences, draws the current one (in place on a terminal, once per line
//! otherwise) and, only when `KURU_OPEN_MARKERS=1`, writes open-time marker
//! lines for the timing harness. Nothing here can fail or delay the open.

use std::borrow::Cow;
use std::ffi::OsStr;
use std::future::Future;
use std::io::{self, IsTerminal, Write};
use std::sync::LazyLock;
use std::time::Instant;

use anyhow::Result;
use futures::FutureExt;
use kuru_memory::{MemoryOpenProgress, MemoryOpenStage};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// S1. True for the whole of every open, so it is the default.
pub const OPENING: &str = "Opening this project's memory…";
/// S2. This version's engine is being unpacked or checked on this computer,
/// or this open waits for the process doing so.
pub const GETTING_READY: &str = "Getting Kuru's memory ready on this computer…";
/// S3. The project has no active memory and this open is creating it,
/// including an import or an interrupted setup being finished.
pub const CREATING: &str = "Creating this project's memory…";
/// S4. An existing project's memory is being upgraded to this version.
pub const UPGRADING: &str = "Upgrading this project's memory…";
/// S5. Another process holds this project's memory, often the previous
/// command's service while it closes.
pub const WAITING: &str = "Waiting for another copy of Kuru to finish with this project's memory…";

/// Every sentence a memory open can show, and nothing else.
pub const SENTENCES: [&str; 5] = [OPENING, GETTING_READY, CREATING, UPGRADING, WAITING];

/// After an open whose engine installation kept a receipted stage.
pub const LEFTOVER_RETRY: &str =
    "Kuru could not remove some leftover setup files; it will try again on a later start.";
/// The same, when no receipt could be written and the cache is the default.
pub const LEFTOVER_MANUAL: &str = "Kuru could not remove some leftover setup files from the tools folder of its data directory, and will not retry. Remove them by hand when no copy of Kuru is running.";
/// The same, when `memory.cache_dir` selects the cache.
pub const LEFTOVER_MANUAL_CONFIGURED: &str = "Kuru could not remove some leftover setup files from the folder set by memory.cache_dir, and will not retry. Remove them by hand when no copy of Kuru is running.";

/// Set to exactly `1` to write open-time marker lines to standard error.
pub const MARKERS_ENV: &str = "KURU_OPEN_MARKERS";
/// The start of every marker line: `kuru-open-marker v1 <event> <monotonic_ns>`.
pub const MARKER_PREFIX: &str = "kuru-open-marker v1 ";

const ELLIPSIS: &str = "…";

/// One monotonic anchor per process; every marker is measured from it.
static ANCHOR: LazyLock<Instant> = LazyLock::new(Instant::now);

/// Fix the marker clock's anchor, so `open-start` is close to zero.
pub(crate) fn start_clock() {
    LazyLock::force(&ANCHOR);
}

/// Markers are written only when the variable is exactly `1`.
pub(crate) fn markers_enabled(value: Option<&OsStr>) -> bool {
    value == Some(OsStr::new("1"))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Sentence {
    Opening,
    GettingReady,
    Creating,
    Upgrading,
    Waiting,
}

impl Sentence {
    pub(crate) const fn text(self) -> &'static str {
        match self {
            Self::Opening => OPENING,
            Self::GettingReady => GETTING_READY,
            Self::Creating => CREATING,
            Self::Upgrading => UPGRADING,
            Self::Waiting => WAITING,
        }
    }
}

/// The sentence a stage begins, or `None` to keep the current one.
///
/// A specific sentence appears only from a stage reported where its work
/// begins, and gives way to the stage that begins different work. Stages
/// that begin nothing new, including any stage added later, keep it.
pub(crate) fn next_sentence(current: Sentence, stage: MemoryOpenStage) -> Option<Sentence> {
    match stage {
        // R1
        MemoryOpenStage::WaitingForProjectOwnership => Some(Sentence::Waiting),
        // R2
        MemoryOpenStage::WaitingForRuntimeCache | MemoryOpenStage::ExtractingEmbeddedRuntime => {
            Some(Sentence::GettingReady)
        }
        // R3
        MemoryOpenStage::VerifyingRuntimeCache
        | MemoryOpenStage::PreparingDatabase
        | MemoryOpenStage::StartingMemoryService => Some(Sentence::Opening),
        // R4: the cold check of a just-unpacked engine is still getting it
        // ready; otherwise (a warm cache or a configured binary) it is not.
        MemoryOpenStage::CheckingRuntimeVersion => {
            (current != Sentence::GettingReady).then_some(Sentence::Opening)
        }
        // R5
        MemoryOpenStage::CreatingDatabase => Some(Sentence::Creating),
        // R6: upgrading a recovered stage is still part of creating.
        MemoryOpenStage::UpgradingDatabase => {
            (current != Sentence::Creating).then_some(Sentence::Upgrading)
        }
        // R7 (`OpeningDatabase`), R8 (`Ready`: the line is erased when the
        // open returns) and R9 (the retained-install notices and any stage
        // added later) keep the sentence.
        _ => None,
    }
}

/// The notice printed after a successful open whose engine installation kept
/// a stage it could not remove.
pub(crate) fn leftover_notice(
    stage: MemoryOpenStage,
    configured_cache: bool,
) -> Option<&'static str> {
    match stage {
        MemoryOpenStage::RetainedInstallStage => Some(LEFTOVER_RETRY),
        MemoryOpenStage::RetainedUnreceiptedInstallStage if configured_cache => {
            Some(LEFTOVER_MANUAL_CONFIGURED)
        }
        MemoryOpenStage::RetainedUnreceiptedInstallStage => Some(LEFTOVER_MANUAL),
        _ => None,
    }
}

/// The text drawn for `sentence` within `budget` columns.
///
/// Widths use the wide reading of ambiguous characters (the final `…` is the
/// only one), an upper bound, so a drawn line never wraps and an erase always
/// covers it. A shortened sentence keeps its trailing ellipsis; with no room
/// for one character and the ellipsis, nothing is drawn.
fn fit(sentence: &'static str, budget: Option<usize>) -> Cow<'static, str> {
    let Some(budget) = budget else {
        return Cow::Borrowed(sentence);
    };
    if sentence.width_cjk() <= budget {
        return Cow::Borrowed(sentence);
    }
    let ellipsis = ELLIPSIS.width_cjk();
    if budget < 1 + ellipsis {
        return Cow::Borrowed("");
    }
    let mut used = 0;
    let mut end = 0;
    for (index, character) in sentence.char_indices() {
        let width = character.width_cjk().unwrap_or(0);
        if used + width + ellipsis > budget {
            break;
        }
        used += width;
        end = index + character.len_utf8();
    }
    let prefix = sentence[..end].trim_end_matches(' ');
    if prefix.is_empty() {
        return Cow::Borrowed("");
    }
    Cow::Owned(format!("{prefix}{ELLIPSIS}"))
}

/// The real standard-error sink: each write locks stderr for the call.
struct StderrSink;

impl Write for StderrSink {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        io::stderr().lock().write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        io::stderr().lock().flush()
    }
}

/// The interactive terminal, when an interactive session's stderr is not one.
struct StdoutSink;

impl Write for StdoutSink {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        io::stdout().lock().write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        io::stdout().lock().flush()
    }
}

/// The terminal's width in columns, or `None` when unknown.
pub(crate) type Columns = Box<dyn Fn() -> Option<u16> + Send>;

pub(crate) enum Layout {
    /// Each new sentence once, whole, on its own line; nothing at ready.
    Lines,
    /// One line rewritten in place and erased when the open ends.
    /// `shares_line` is set when marker lines go to that same terminal.
    Terminal { columns: Columns, shares_line: bool },
}

#[derive(Clone, Copy)]
enum Marker {
    OpenStart,
    WaitingOwnership,
    Ready,
}

impl Marker {
    const fn event(self) -> &'static str {
        match self {
            Self::OpenStart => "open-start",
            Self::WaitingOwnership => "waiting-ownership",
            Self::Ready => "ready",
        }
    }
}

/// Renders the current sentence and the optional marker lines.
///
/// Sentences go to `sentences`; markers and leftover notices always go to
/// `stderr`, so neither ever reaches machine standard output. A failed
/// sentence write switches sentences off; a failed marker or notice write is
/// dropped. No write here can fail the open.
pub(crate) struct ActivityOutput {
    sentences: Box<dyn Write + Send>,
    stderr: Box<dyn Write + Send>,
    layout: Layout,
    markers: bool,
    enabled: bool,
    current: Sentence,
    shown: Option<Sentence>,
    width: usize,
    waited: bool,
    wait_marked: bool,
}

impl ActivityOutput {
    pub(crate) fn new(
        sentences: Box<dyn Write + Send>,
        stderr: Box<dyn Write + Send>,
        layout: Layout,
        markers: bool,
    ) -> Self {
        Self {
            sentences,
            stderr,
            layout,
            markers,
            enabled: true,
            current: Sentence::Opening,
            shown: None,
            width: 0,
            waited: false,
            wait_marked: false,
        }
    }

    /// Standard error on a terminal; the interactive terminal through
    /// standard output when an interactive session's stderr is redirected;
    /// otherwise one line per sentence on standard error.
    pub(crate) fn for_process(interactive: bool, markers: bool) -> Self {
        let columns: Columns =
            Box::new(|| crossterm::terminal::size().ok().map(|(columns, _)| columns));
        if io::stderr().is_terminal() {
            let layout = Layout::Terminal {
                columns,
                shares_line: true,
            };
            Self::new(Box::new(StderrSink), Box::new(StderrSink), layout, markers)
        } else if interactive && io::stdout().is_terminal() {
            let layout = Layout::Terminal {
                columns,
                shares_line: false,
            };
            Self::new(Box::new(StdoutSink), Box::new(StderrSink), layout, markers)
        } else {
            Self::new(
                Box::new(StderrSink),
                Box::new(StderrSink),
                Layout::Lines,
                markers,
            )
        }
    }

    /// Before the open is first polled: the `open-start` marker, then S1.
    pub(crate) fn start(&mut self) {
        self.mark(Marker::OpenStart);
        self.draw();
    }

    /// Record one stage without drawing, so a batch draws once.
    pub(crate) fn apply(&mut self, stage: MemoryOpenStage) {
        if stage == MemoryOpenStage::WaitingForProjectOwnership {
            self.waited = true;
        }
        if let Some(next) = next_sentence(self.current, stage) {
            self.current = next;
        }
    }

    /// Draw the batch's outcome. A wait is marked even when a later stage in
    /// the same batch already replaced its sentence.
    pub(crate) fn render(&mut self) {
        self.mark_wait();
        self.draw();
    }

    /// Memory is ready: erase the line; no ready line is written.
    pub(crate) fn complete(&mut self) {
        self.erase();
        self.mark_wait();
        self.mark(Marker::Ready);
    }

    /// The open failed: erase the line before the error is printed.
    pub(crate) fn abandon(&mut self) {
        self.erase();
        self.mark_wait();
    }

    /// A retained line after a successful open, on standard error only.
    pub(crate) fn notice(&mut self, text: &str) {
        let line = format!("{text}\n");
        let _ = self
            .stderr
            .write_all(line.as_bytes())
            .and_then(|()| self.stderr.flush());
    }

    /// Write one already-formatted piece as a single buffered call, so the
    /// real sink locks once per piece.
    fn write_sentence(&mut self, text: &str) {
        if self
            .sentences
            .write_all(text.as_bytes())
            .and_then(|()| self.sentences.flush())
            .is_err()
        {
            self.enabled = false;
        }
    }

    fn budget(&self) -> Option<usize> {
        match &self.layout {
            Layout::Lines => None,
            Layout::Terminal { columns, .. } => {
                columns().map(|columns| usize::from(columns).saturating_sub(1))
            }
        }
    }

    fn draw(&mut self) {
        if !self.enabled || self.shown == Some(self.current) {
            return;
        }
        let text = self.current.text();
        let line = match self.layout {
            Layout::Lines => format!("{text}\n"),
            Layout::Terminal { .. } => {
                let budget = self.budget();
                let fitted = fit(text, budget);
                let width = fitted.width_cjk();
                // Pad over the widest line so far, but never past the budget.
                let padded = budget.map_or(self.width.max(width), |budget| {
                    self.width.max(width).min(budget)
                });
                self.width = self.width.max(width);
                if padded == 0 {
                    self.shown = Some(self.current);
                    return;
                }
                format!("\r{fitted}{}", " ".repeat(padded - width))
            }
        };
        self.write_sentence(&line);
        self.shown = Some(self.current);
    }

    fn erase(&mut self) {
        if !matches!(self.layout, Layout::Terminal { .. }) {
            return;
        }
        if self.enabled && self.width > 0 {
            let budget = self.budget();
            let width = budget.map_or(self.width, |budget| self.width.min(budget));
            self.write_sentence(&format!("\r{}\r", " ".repeat(width)));
        }
        self.width = 0;
        self.shown = None;
    }

    fn mark_wait(&mut self) {
        if self.waited && !self.wait_marked {
            self.wait_marked = true;
            self.mark(Marker::WaitingOwnership);
        }
    }

    /// One marker line. On the sentence's own terminal the line is erased
    /// first; the next draw writes the sentence again below the marker.
    fn mark(&mut self, marker: Marker) {
        if !self.markers {
            return;
        }
        if matches!(
            self.layout,
            Layout::Terminal {
                shares_line: true,
                ..
            }
        ) {
            self.erase();
        }
        let line = format!(
            "{MARKER_PREFIX}{} {}\n",
            marker.event(),
            ANCHOR.elapsed().as_nanos()
        );
        // A marker that cannot be written is dropped; sentences continue.
        let _ = self
            .stderr
            .write_all(line.as_bytes())
            .and_then(|()| self.stderr.flush());
    }
}

/// A stream of open stages; the observed open's receiver in the product.
pub(crate) trait StageSource {
    fn recv(&mut self) -> impl Future<Output = Option<MemoryOpenStage>>;
}

impl StageSource for MemoryOpenProgress {
    fn recv(&mut self) -> impl Future<Output = Option<MemoryOpenStage>> {
        MemoryOpenProgress::recv(self)
    }
}

/// What the renderer holds back until the open returns.
#[derive(Default)]
struct Held {
    ready: bool,
    retained: Option<MemoryOpenStage>,
}

impl Held {
    fn take(&mut self, stage: MemoryOpenStage, output: &mut ActivityOutput) {
        match stage {
            MemoryOpenStage::Ready => self.ready = true,
            MemoryOpenStage::RetainedInstallStage
            | MemoryOpenStage::RetainedUnreceiptedInstallStage => self.retained = Some(stage),
            stage => output.apply(stage),
        }
    }
}

/// Run `opening` to completion while showing its stages through `output`.
///
/// Stages already waiting are applied together and drawn once. Stages that
/// arrive with the completed open describe finished work and are not drawn.
/// The open's result is returned unchanged.
pub(crate) async fn drive<T>(
    opening: impl Future<Output = Result<T>>,
    mut progress: impl StageSource,
    output: &mut ActivityOutput,
    configured_cache: bool,
) -> Result<T> {
    let mut opening = Box::pin(opening);
    output.start();
    let mut held = Held::default();
    let mut progress_open = true;
    let result = loop {
        tokio::select! {
            biased;
            stage = progress.recv(), if progress_open => match stage {
                Some(stage) => {
                    held.take(stage, output);
                    while let Some(Some(stage)) = progress.recv().now_or_never() {
                        held.take(stage, output);
                    }
                    output.render();
                }
                None => progress_open = false,
            },
            result = &mut opening => break result,
        }
    };
    // Dropping the open drops its reporter, which ends the stream.
    drop(opening);
    while let Some(stage) = progress.recv().await {
        held.take(stage, output);
    }
    match result {
        Ok(value) => {
            // Ready is reported only with a completed usable store. Keeping
            // this check makes a future stage unable to stand in for success.
            debug_assert!(held.ready, "successful observed open must report ready");
            output.complete();
            // Only after the erase and only on success: an abandoned open
            // never prints this notice.
            if let Some(text) = held
                .retained
                .and_then(|stage| leftover_notice(stage, configured_cache))
            {
                output.notice(text);
            }
            Ok(value)
        }
        Err(error) => {
            output.abandon();
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc;

    use MemoryOpenStage as Stage;

    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl Buffer {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    impl Write for Buffer {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Fails every write and counts the attempts.
    #[derive(Clone, Default)]
    struct Failing(Arc<AtomicUsize>);

    impl Write for Failing {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(io::Error::other("fixture sink failure"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl StageSource for mpsc::Receiver<MemoryOpenStage> {
        fn recv(&mut self) -> impl Future<Output = Option<MemoryOpenStage>> {
            mpsc::Receiver::recv(self)
        }
    }

    fn lines(markers: bool) -> (ActivityOutput, Buffer, Buffer) {
        let sentences = Buffer::default();
        let stderr = Buffer::default();
        let output = ActivityOutput::new(
            Box::new(sentences.clone()),
            Box::new(stderr.clone()),
            Layout::Lines,
            markers,
        );
        (output, sentences, stderr)
    }

    /// A terminal of `columns` that is also standard error.
    fn terminal(columns: Option<u16>, markers: bool) -> (ActivityOutput, Buffer) {
        let screen = Buffer::default();
        let output = ActivityOutput::new(
            Box::new(screen.clone()),
            Box::new(screen.clone()),
            Layout::Terminal {
                columns: Box::new(move || columns),
                shares_line: true,
            },
            markers,
        );
        (output, screen)
    }

    fn feed(output: &mut ActivityOutput, stages: &[Stage]) {
        for &stage in stages {
            output.apply(stage);
            output.render();
        }
    }

    /// The sentence after each stage, starting from S1.
    fn sentences_after(stages: &[Stage]) -> Vec<Sentence> {
        let mut current = Sentence::Opening;
        let mut seen = vec![current];
        for &stage in stages {
            if let Some(next) = next_sentence(current, stage) {
                current = next;
            }
            seen.push(current);
        }
        seen
    }

    /// The visible screen after `bytes`: `\r` returns to column 0, `\n`
    /// starts a new line, every other character takes one cell.
    fn visible_rows(bytes: &str) -> Vec<String> {
        let mut rows = vec![Vec::<char>::new()];
        let mut column = 0;
        for character in bytes.chars() {
            match character {
                '\r' => column = 0,
                '\n' => {
                    rows.push(Vec::new());
                    column = 0;
                }
                character => {
                    let row = rows.last_mut().unwrap();
                    if column < row.len() {
                        row[column] = character;
                    } else {
                        row.push(character);
                    }
                    column += 1;
                }
            }
        }
        rows.into_iter().map(String::from_iter).collect()
    }

    /// The marker events and times on the visible rows of `text`.
    fn markers_in(text: &str) -> Vec<(String, u128)> {
        visible_rows(text)
            .iter()
            .filter_map(|row| row.trim_end().strip_prefix(MARKER_PREFIX))
            .map(|rest| {
                let (event, ns) = rest.split_once(' ').unwrap();
                (event.to_owned(), ns.parse().unwrap())
            })
            .collect()
    }

    const BASE_SEQUENCES: [&[Stage]; 7] = [
        // A cold cache and a new project.
        &[
            Stage::StartingMemoryService,
            Stage::WaitingForRuntimeCache,
            Stage::ExtractingEmbeddedRuntime,
            Stage::CheckingRuntimeVersion,
            Stage::PreparingDatabase,
            Stage::CreatingDatabase,
            Stage::OpeningDatabase,
        ],
        // A warm cache and an existing project.
        &[
            Stage::StartingMemoryService,
            Stage::VerifyingRuntimeCache,
            Stage::CheckingRuntimeVersion,
            Stage::PreparingDatabase,
            Stage::OpeningDatabase,
        ],
        // A configured engine binary.
        &[
            Stage::CheckingRuntimeVersion,
            Stage::PreparingDatabase,
            Stage::OpeningDatabase,
        ],
        // An upgrade of a recovered stage, inside creation.
        &[
            Stage::PreparingDatabase,
            Stage::CreatingDatabase,
            Stage::OpeningDatabase,
            Stage::UpgradingDatabase,
        ],
        // An upgrade after an engine change.
        &[
            Stage::StartingMemoryService,
            Stage::ExtractingEmbeddedRuntime,
            Stage::CheckingRuntimeVersion,
            Stage::PreparingDatabase,
            Stage::OpeningDatabase,
            Stage::UpgradingDatabase,
        ],
        // A wait for a previous owner, then this client's own service.
        &[
            Stage::WaitingForProjectOwnership,
            Stage::StartingMemoryService,
            Stage::VerifyingRuntimeCache,
            Stage::PreparingDatabase,
        ],
        // A read-only inspection whose own open unpacks the engine.
        &[
            Stage::WaitingForProjectOwnership,
            Stage::ExtractingEmbeddedRuntime,
            Stage::CheckingRuntimeVersion,
            Stage::PreparingDatabase,
            Stage::OpeningDatabase,
        ],
    ];

    #[test]
    fn every_sentence_is_plain_and_ends_with_one_ellipsis() {
        for text in SENTENCES {
            assert_eq!(text.chars().last(), Some('…'), "{text}");
            assert_eq!(text.matches('…').count(), 1, "{text}");
            assert!(!text.contains('\u{2019}'), "apostrophes are ASCII: {text}");
            assert!(!text.contains(':'), "no label: {text}");
        }
        for text in SENTENCES.into_iter().chain([
            LEFTOVER_RETRY,
            LEFTOVER_MANUAL,
            LEFTOVER_MANUAL_CONFIGURED,
        ]) {
            assert!(!text.contains("Memory:"), "{text}");
        }
        assert_eq!(
            WAITING,
            "Waiting for another copy of Kuru to finish with this project's memory…"
        );
        let distinct: std::collections::BTreeSet<_> = SENTENCES.into_iter().collect();
        assert_eq!(distinct.len(), SENTENCES.len());
    }

    #[test]
    fn each_rule_maps_its_stage() {
        let all = [
            Sentence::Opening,
            Sentence::GettingReady,
            Sentence::Creating,
            Sentence::Upgrading,
            Sentence::Waiting,
        ];
        for current in all {
            // R1 to R3 and R5 do not depend on the current sentence.
            for (stage, expected) in [
                (Stage::WaitingForProjectOwnership, Sentence::Waiting),
                (Stage::WaitingForRuntimeCache, Sentence::GettingReady),
                (Stage::ExtractingEmbeddedRuntime, Sentence::GettingReady),
                (Stage::VerifyingRuntimeCache, Sentence::Opening),
                (Stage::PreparingDatabase, Sentence::Opening),
                (Stage::StartingMemoryService, Sentence::Opening),
                (Stage::CreatingDatabase, Sentence::Creating),
            ] {
                assert_eq!(next_sentence(current, stage), Some(expected), "{stage:?}");
            }
            // R4
            let checking = next_sentence(current, Stage::CheckingRuntimeVersion);
            if current == Sentence::GettingReady {
                assert_eq!(checking, None);
            } else {
                assert_eq!(checking, Some(Sentence::Opening));
            }
            // R6
            let upgrading = next_sentence(current, Stage::UpgradingDatabase);
            if current == Sentence::Creating {
                assert_eq!(upgrading, None);
            } else {
                assert_eq!(upgrading, Some(Sentence::Upgrading));
            }
            // R7, R8 and R9. A stage added to memory later cannot be built
            // here (the enum is non-exhaustive); the retained-install stages
            // take the same fallback arm.
            for stage in [
                Stage::OpeningDatabase,
                Stage::Ready,
                Stage::RetainedInstallStage,
                Stage::RetainedUnreceiptedInstallStage,
            ] {
                assert_eq!(next_sentence(current, stage), None, "{stage:?}");
            }
        }
    }

    #[test]
    fn named_sequences_show_only_the_work_that_began() {
        use Sentence::{Creating, GettingReady, Opening, Upgrading, Waiting};
        let expected: [&[Sentence]; 7] = [
            &[
                Opening,
                Opening,
                GettingReady,
                GettingReady,
                GettingReady,
                Opening,
                Creating,
                Creating,
            ],
            &[Opening; 6],
            &[Opening; 4],
            &[Opening, Opening, Creating, Creating, Creating],
            &[
                Opening,
                Opening,
                GettingReady,
                GettingReady,
                Opening,
                Opening,
                Upgrading,
            ],
            &[Opening, Waiting, Opening, Opening, Opening],
            &[
                Opening,
                Waiting,
                GettingReady,
                GettingReady,
                Opening,
                Opening,
            ],
        ];
        for (stages, expected) in BASE_SEQUENCES.iter().zip(expected) {
            assert_eq!(sentences_after(stages), expected, "{stages:?}");
        }
    }

    /// The sentences shown never depend on how many internal steps memory
    /// reports: inserting stages that begin no new work, or repeating one,
    /// anywhere in a sequence leaves the written lines unchanged.
    #[test]
    fn inserted_internal_steps_never_change_the_sentences_shown() {
        let rendered = |stages: &[Stage]| {
            let (mut output, sentences, _) = lines(false);
            output.start();
            feed(&mut output, stages);
            sentences.text()
        };
        for base in BASE_SEQUENCES {
            let expected = rendered(base);
            for line in expected.lines() {
                assert!(SENTENCES.contains(&line), "{line:?}");
            }
            for position in 0..=base.len() {
                let mut fillers = vec![
                    Stage::OpeningDatabase,
                    Stage::RetainedInstallStage,
                    Stage::RetainedUnreceiptedInstallStage,
                ];
                if position > 0 {
                    fillers.push(base[position - 1]);
                }
                for filler in fillers {
                    for count in 1..=3 {
                        let mut stages = base.to_vec();
                        stages.splice(position..position, std::iter::repeat_n(filler, count));
                        assert_eq!(rendered(&stages), expected, "{stages:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_terminal_rewrites_one_line_padded_by_its_widest_wide_width() {
        let (mut output, screen) = terminal(Some(200), false);
        output.start();
        feed(
            &mut output,
            &[
                Stage::WaitingForProjectOwnership,
                Stage::StartingMemoryService,
            ],
        );
        let opening = OPENING.width_cjk();
        let waiting = WAITING.width_cjk();
        assert_eq!(opening, OPENING.chars().count() + 1, "… reads as two");
        assert_eq!(
            screen.text(),
            format!(
                "\r{OPENING}\r{WAITING}\r{OPENING}{}",
                " ".repeat(waiting - opening)
            )
        );
        // The same sentence is not written again.
        feed(
            &mut output,
            &[Stage::PreparingDatabase, Stage::OpeningDatabase],
        );
        assert!(screen.text().ends_with(&" ".repeat(waiting - opening)));
        output.complete();
        assert!(
            screen
                .text()
                .ends_with(&format!("\r{}\r", " ".repeat(waiting)))
        );
        assert_eq!(screen.text().matches('\n').count(), 0, "no ready line");
    }

    #[test]
    fn a_shortened_sentence_keeps_its_ellipsis_and_never_wraps() {
        for text in SENTENCES {
            for columns in 1..=120_u16 {
                let budget = usize::from(columns) - 1;
                let fitted = fit(text, Some(budget));
                assert!(fitted.width_cjk() <= budget, "{columns}: {fitted:?}");
                if fitted.is_empty() {
                    assert!(columns < 4, "{columns}: {text}");
                } else {
                    assert!(fitted.ends_with('…'), "{columns}: {fitted:?}");
                    assert!(!fitted.ends_with(" …"), "{columns}: {fitted:?}");
                    let kept = fitted.trim_end_matches('…');
                    assert!(text.starts_with(kept), "{columns}: {fitted:?}");
                }
            }
        }
        assert_eq!(fit(WAITING, Some(19)), "Waiting for anoth…");
        assert_eq!(fit(OPENING, Some(19)), "Opening this proj…");
        assert_eq!(fit(OPENING, Some(15)), "Opening this…");
        assert_eq!(fit(OPENING, Some(3)), "O…");
        assert_eq!(fit(OPENING, Some(2)), "");
        assert_eq!(fit(OPENING, Some(1)), "");
        assert_eq!(fit(OPENING, None), OPENING);
    }

    #[test]
    fn a_narrow_terminal_draws_within_its_width_and_nothing_without_room() {
        for columns in [20, 3, 2] {
            let (mut output, screen) = terminal(Some(columns), false);
            output.start();
            feed(&mut output, &[Stage::WaitingForProjectOwnership]);
            output.complete();
            let budget = usize::from(columns) - 1;
            for piece in screen.text().split('\r') {
                assert!(piece.width_cjk() <= budget, "{columns}: {piece:?}");
            }
            if columns < 4 {
                assert_eq!(screen.text(), "", "{columns}");
            }
        }
        let (mut output, screen) = terminal(None, false);
        output.start();
        assert_eq!(screen.text(), format!("\r{OPENING}"), "unknown width");
    }

    #[test]
    fn a_failed_open_erases_the_line() {
        let (mut output, screen) = terminal(Some(120), false);
        output.start();
        output.abandon();
        assert_eq!(
            screen.text(),
            format!("\r{OPENING}\r{}\r", " ".repeat(OPENING.width_cjk()))
        );
    }

    #[test]
    fn without_a_terminal_each_new_sentence_is_one_whole_line_and_ready_writes_nothing() {
        let (mut output, sentences, stderr) = lines(false);
        output.start();
        feed(
            &mut output,
            &[
                Stage::StartingMemoryService,
                Stage::ExtractingEmbeddedRuntime,
                Stage::CheckingRuntimeVersion,
                Stage::PreparingDatabase,
                Stage::CreatingDatabase,
                Stage::OpeningDatabase,
            ],
        );
        output.complete();
        assert_eq!(
            sentences.text(),
            format!("{OPENING}\n{GETTING_READY}\n{OPENING}\n{CREATING}\n")
        );
        assert_eq!(stderr.text(), "");
    }

    #[test]
    fn a_failed_sentence_write_switches_sentences_off() {
        let failing = Failing::default();
        let mut output = ActivityOutput::new(
            Box::new(failing.clone()),
            Box::new(Buffer::default()),
            Layout::Lines,
            false,
        );
        output.start();
        feed(&mut output, &[Stage::WaitingForProjectOwnership]);
        output.complete();
        assert!(!output.enabled);
        assert_eq!(failing.0.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn the_unreceipted_notice_names_the_configured_cache() {
        assert_eq!(
            leftover_notice(Stage::RetainedInstallStage, false),
            Some(LEFTOVER_RETRY)
        );
        assert_eq!(
            leftover_notice(Stage::RetainedInstallStage, true),
            Some(LEFTOVER_RETRY)
        );
        assert_eq!(
            leftover_notice(Stage::RetainedUnreceiptedInstallStage, false),
            Some(LEFTOVER_MANUAL)
        );
        assert_eq!(
            leftover_notice(Stage::RetainedUnreceiptedInstallStage, true),
            Some(LEFTOVER_MANUAL_CONFIGURED)
        );
        assert!(LEFTOVER_MANUAL_CONFIGURED.contains("memory.cache_dir"));
        assert!(!LEFTOVER_MANUAL_CONFIGURED.contains("tools folder"));
        assert!(!LEFTOVER_RETRY.contains("will not retry"));
        assert_eq!(leftover_notice(Stage::Ready, true), None);
    }

    #[test]
    fn markers_are_enabled_only_by_exactly_one() {
        assert!(markers_enabled(Some(OsStr::new("1"))));
        for value in ["", "0", "true", " 1", "1 ", "01", "yes"] {
            assert!(!markers_enabled(Some(OsStr::new(value))), "{value:?}");
        }
        assert!(!markers_enabled(None));
    }

    #[test]
    fn without_markers_no_marker_is_written_anywhere() {
        let (mut output, sentences, stderr) = lines(false);
        output.start();
        feed(&mut output, &[Stage::WaitingForProjectOwnership]);
        output.complete();
        let (mut output, screen) = terminal(Some(80), false);
        output.start();
        feed(&mut output, &[Stage::WaitingForProjectOwnership]);
        output.complete();
        for text in [sentences.text(), stderr.text(), screen.text()] {
            assert!(!text.contains("kuru-open-marker"), "{text:?}");
        }
    }

    #[test]
    fn markers_go_to_stderr_only_and_never_to_the_sentence_sink() {
        // An interactive session whose stderr is redirected: sentences go to
        // the terminal through standard output, markers to stderr.
        let stdout = Buffer::default();
        let stderr = Buffer::default();
        let mut output = ActivityOutput::new(
            Box::new(stdout.clone()),
            Box::new(stderr.clone()),
            Layout::Terminal {
                columns: Box::new(|| Some(120)),
                shares_line: false,
            },
            true,
        );
        output.start();
        feed(&mut output, &[Stage::WaitingForProjectOwnership]);
        output.complete();
        assert!(!stdout.text().contains("kuru-open-marker"));
        assert!(stdout.text().contains(OPENING));
        let events: Vec<_> = markers_in(&stderr.text())
            .into_iter()
            .map(|(event, _)| event)
            .collect();
        assert_eq!(events, ["open-start", "waiting-ownership", "ready"]);
        assert!(
            stderr
                .text()
                .lines()
                .all(|line| line.starts_with(MARKER_PREFIX))
        );
    }

    #[test]
    fn marker_lines_hold_only_the_marker_and_never_share_a_sentence_line() {
        let (mut output, screen) = terminal(Some(120), true);
        output.start();
        feed(
            &mut output,
            &[
                Stage::WaitingForProjectOwnership,
                Stage::StartingMemoryService,
                Stage::PreparingDatabase,
                Stage::CreatingDatabase,
            ],
        );
        output.complete();
        let text = screen.text();
        let markers = markers_in(&text);
        let events: Vec<_> = markers.iter().map(|(event, _)| event.as_str()).collect();
        assert_eq!(events, ["open-start", "waiting-ownership", "ready"]);
        assert!(markers.windows(2).all(|pair| pair[0].1 <= pair[1].1));
        let rows = visible_rows(&text);
        for row in &rows {
            if row.contains("kuru-open-marker") {
                let row = row.trim_end();
                assert!(row.starts_with(MARKER_PREFIX), "{row:?}");
                let (_, ns) = row[MARKER_PREFIX.len()..].split_once(' ').unwrap();
                assert!(ns.bytes().all(|byte| byte.is_ascii_digit()), "{row:?}");
            }
        }
        assert_eq!(rows.last().map(|row| row.trim()), Some(""), "{rows:?}");
        // The sentence after the waiting marker was drawn below it.
        assert!(text.contains(&format!("\n\r{WAITING}")), "{text:?}");
    }

    #[test]
    fn the_waiting_marker_appears_only_when_the_wait_happened() {
        let (mut output, _, stderr) = lines(true);
        output.start();
        feed(
            &mut output,
            &[Stage::StartingMemoryService, Stage::PreparingDatabase],
        );
        output.complete();
        let events: Vec<_> = markers_in(&stderr.text())
            .into_iter()
            .map(|(event, _)| event)
            .collect();
        assert_eq!(events, ["open-start", "ready"]);

        let (mut output, _, stderr) = lines(true);
        output.start();
        feed(&mut output, &[Stage::WaitingForProjectOwnership]);
        output.abandon();
        let events: Vec<_> = markers_in(&stderr.text())
            .into_iter()
            .map(|(event, _)| event)
            .collect();
        assert_eq!(
            events,
            ["open-start", "waiting-ownership"],
            "no ready on failure"
        );
    }

    #[tokio::test]
    async fn queued_stages_draw_once_and_a_short_wait_keeps_only_its_marker() {
        let (sender, receiver) = mpsc::channel(16);
        for stage in [
            Stage::WaitingForProjectOwnership,
            Stage::StartingMemoryService,
            Stage::PreparingDatabase,
        ] {
            sender.try_send(stage).unwrap();
        }
        let opening = async move {
            tokio::task::yield_now().await;
            sender.send(Stage::Ready).await.unwrap();
            Ok::<_, anyhow::Error>(7)
        };
        let (mut output, sentences, stderr) = lines(true);
        let value = drive(opening, receiver, &mut output, false).await.unwrap();
        assert_eq!(value, 7);
        assert_eq!(sentences.text(), format!("{OPENING}\n"));
        let events: Vec<_> = markers_in(&stderr.text())
            .into_iter()
            .map(|(event, _)| event)
            .collect();
        assert_eq!(events, ["open-start", "waiting-ownership", "ready"]);
    }

    #[tokio::test]
    async fn stages_reported_during_the_open_are_drawn_and_late_ones_are_not() {
        let (sender, receiver) = mpsc::channel(16);
        let opening = async move {
            sender.send(Stage::ExtractingEmbeddedRuntime).await.unwrap();
            tokio::task::yield_now().await;
            sender.send(Stage::CreatingDatabase).await.unwrap();
            tokio::task::yield_now().await;
            // Reported with the completed open: finished work.
            sender.send(Stage::PreparingDatabase).await.unwrap();
            sender
                .send(Stage::RetainedUnreceiptedInstallStage)
                .await
                .unwrap();
            sender.send(Stage::Ready).await.unwrap();
            Ok::<_, anyhow::Error>(())
        };
        let (mut output, sentences, stderr) = lines(false);
        drive(opening, receiver, &mut output, true).await.unwrap();
        assert_eq!(
            sentences.text(),
            format!("{OPENING}\n{GETTING_READY}\n{CREATING}\n")
        );
        assert_eq!(stderr.text(), format!("{LEFTOVER_MANUAL_CONFIGURED}\n"));
    }

    #[tokio::test]
    async fn a_failed_open_returns_its_error_erased_and_without_a_notice() {
        let (sender, receiver) = mpsc::channel(16);
        let opening = async move {
            sender.send(Stage::RetainedInstallStage).await.unwrap();
            tokio::task::yield_now().await;
            Err::<(), _>(anyhow::anyhow!("fixture open failure"))
        };
        let (mut output, screen) = terminal(Some(120), true);
        let error = drive(opening, receiver, &mut output, false)
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "fixture open failure");
        let text = screen.text();
        assert!(text.ends_with(&format!("\r{}\r", " ".repeat(OPENING.width_cjk()))));
        assert!(!text.contains(LEFTOVER_RETRY));
        let events: Vec<_> = markers_in(&text).into_iter().map(|(e, _)| e).collect();
        assert_eq!(events, ["open-start"]);
    }

    /// Feedback never fails the open: with every sentence, marker and notice
    /// write failing, the open's own result comes back unchanged, and a
    /// failed sentence sink is not written again.
    #[tokio::test]
    async fn failing_output_never_changes_the_open_result() {
        let (sender, receiver) = mpsc::channel(16);
        let opening = async move {
            sender
                .send(Stage::WaitingForProjectOwnership)
                .await
                .unwrap();
            tokio::task::yield_now().await;
            sender.send(Stage::CreatingDatabase).await.unwrap();
            tokio::task::yield_now().await;
            sender.send(Stage::RetainedInstallStage).await.unwrap();
            sender.send(Stage::Ready).await.unwrap();
            Ok::<_, anyhow::Error>("store")
        };
        let sentences = Failing::default();
        let stderr = Failing::default();
        let mut output = ActivityOutput::new(
            Box::new(sentences.clone()),
            Box::new(stderr.clone()),
            Layout::Terminal {
                columns: Box::new(|| Some(80)),
                shares_line: true,
            },
            true,
        );
        let value = drive(opening, receiver, &mut output, false).await.unwrap();
        assert_eq!(value, "store");
        assert_eq!(sentences.0.load(Ordering::SeqCst), 1);
        // open-start, waiting-ownership, ready and the notice were each tried.
        assert_eq!(stderr.0.load(Ordering::SeqCst), 4);
    }
}
