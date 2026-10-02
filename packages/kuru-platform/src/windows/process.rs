//! Explicit Windows process creation. Owned Jobs are assigned atomically by
//! CreateProcessW, with a separate allowlist for inherited handles.
//!
//! The allowlist constrains children created here. Unrelated legacy spawners
//! using broad inheritance can still inherit temporarily inheritable handles;
//! concurrent callers requiring isolation must use this boundary consistently.

use super::pipe;
use super::pipe::Pipe;
mod command;
mod image;
pub use command::{
    configured_command, environment_key_eq, merge_environment, resolve_executable, system_directory,
};
pub use image::{CurrentImage, current_image};
use std::{
    cmp::Ordering,
    ffi::{OsStr, OsString, c_void},
    fmt, io,
    mem::{self, size_of},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::ExitStatusExt,
    },
    path::PathBuf,
    process::ExitStatus,
    ptr,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{
        DUPLICATE_SAME_ACCESS, DuplicateHandle, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER,
        ERROR_MORE_DATA, FILETIME, GENERIC_READ, GENERIC_WRITE, HANDLE, HANDLE_FLAG_INHERIT,
        INVALID_HANDLE_VALUE, SetHandleInformation, WAIT_OBJECT_0, WAIT_TIMEOUT,
    },
    Globalization::{CSTR_EQUAL, CSTR_LESS_THAN, CompareStringOrdinal},
    Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING},
    System::{
        Console::{
            CTRL_BREAK_EVENT, GenerateConsoleCtrlEvent, GetStdHandle, STD_ERROR_HANDLE,
            STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
        },
        JobObjects::{
            CreateJobObjectW, IsProcessInJob, JOB_OBJECT_LIMIT_BREAKAWAY_OK,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
            JOBOBJECT_BASIC_PROCESS_ID_LIST, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectBasicAccountingInformation, JobObjectBasicProcessIdList,
            JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
            TerminateJobObject,
        },
        Memory::{GetProcessHeap, HeapAlloc, HeapFree},
        ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
        Threading::{
            CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_CONSOLE, CREATE_NEW_PROCESS_GROUP,
            CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
            EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess, GetExitCodeProcess, GetProcessId,
            GetProcessTimes, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
            OpenProcess, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST,
            PROCESS_INFORMATION, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
            PROCESS_SYNCHRONIZE, QueryFullProcessImageNameW, STARTF_USESHOWWINDOW,
            STARTF_USESTDHANDLES, STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute,
            WaitForSingleObject,
        },
    },
    UI::WindowsAndMessaging::SW_HIDE,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Lifetime {
    /// Child and descendants belong to a non-inheritable kill-on-close Job.
    #[default]
    OwnedJob,
    /// Caller supplies an explicit lifetime protocol and must await cleanup.
    /// Drop closes the process handle without terminating this child.
    TrustedSupervisor,
    /// A service independent of its starter requests native Job breakaway. If
    /// an existing nonpermitting Job denies that exact create, the service may
    /// remain inside that outer lifetime boundary while retaining its own
    /// process handle and cleanup protocol.
    IndependentService,
    /// Native fixture only: a kill-on-close Job permitting explicit breakaway.
    #[cfg(feature = "test-support")]
    FixtureBreakawayJob,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Console {
    #[default]
    Inherit,
    NewProcessGroup,
    PrivateHidden,
}

#[derive(Default)]
pub enum Stdio {
    #[default]
    Null,
    Pipe,
    Handle(OwnedHandle),
}

/// Select a caller's standard stream without passing an ambient raw handle to a consumer.
pub enum StandardStream {
    Input,
    Output,
    Error,
}

/// A private duplicate is included in this spawn's explicit handle allowlist.
/// A caller without a console/redirected stream supplies NUL instead.
pub fn inherited_stdio(stream: StandardStream) -> io::Result<Stdio> {
    let channel = match stream {
        StandardStream::Input => STD_INPUT_HANDLE,
        StandardStream::Output => STD_OUTPUT_HANDLE,
        StandardStream::Error => STD_ERROR_HANDLE,
    };
    // SAFETY: GetStdHandle returns a borrowed process-owned standard handle.
    let handle = unsafe { GetStdHandle(channel) };
    if handle.is_null() {
        return Ok(Stdio::Null);
    }
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let mut duplicate = ptr::null_mut();
    // SAFETY: both process arguments are current-process pseudo handles, the
    // original remains borrowed, and success initializes one owned duplicate.
    if unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            handle,
            GetCurrentProcess(),
            &mut duplicate,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful DuplicateHandle transfers this newly created handle.
    Ok(Stdio::Handle(unsafe {
        OwnedHandle::from_raw_handle(duplicate)
    }))
}

/// Command parsing is explicit. Ordinary argv never gains interpreter syntax.
#[derive(Default)]
pub enum CommandSyntax {
    #[default]
    Argv,
    /// Execute the resolved executable/script and literal args through system cmd.
    CmdInvocation { switches: Vec<OsString> },
    /// Caller deliberately supplied cmd source, not an ordinary argument vector.
    CmdSource {
        switches: Vec<OsString>,
        source: OsString,
    },
}

/// Complete creation intent. Arguments/environment are deliberately not Debug:
/// they can contain caller credentials. Environment never inherits implicitly.
pub struct NativeSpawnSpec {
    pub executable: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    pub environment: Vec<(OsString, OsString)>,
    pub stdin: Stdio,
    pub stdout: Stdio,
    pub stderr: Stdio,
    pub inherited: Vec<OwnedHandle>,
    pub lifetime: Lifetime,
    pub console: Console,
    pub syntax: CommandSyntax,
}

impl NativeSpawnSpec {
    pub fn new(executable: PathBuf, cwd: PathBuf) -> Self {
        Self {
            executable,
            args: Vec::new(),
            cwd,
            environment: Vec::new(),
            stdin: Stdio::Null,
            stdout: Stdio::Null,
            stderr: Stdio::Null,
            inherited: Vec::new(),
            lifetime: Lifetime::OwnedJob,
            console: Console::Inherit,
            syntax: CommandSyntax::Argv,
        }
    }

    pub async fn spawn(self) -> io::Result<NativeChild> {
        if !self.executable.is_absolute() || !self.cwd.is_absolute() {
            return Err(invalid(
                "native executable and working directory must be absolute",
            ));
        }
        if matches!(self.syntax, CommandSyntax::Argv)
            && self.executable.extension().is_some_and(|extension| {
                extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
            })
        {
            return Err(invalid(
                "batch execution requires an explicit consumer shell policy",
            ));
        }
        let (executable, command_line) = match &self.syntax {
            CommandSyntax::Argv => (
                wide(self.executable.as_os_str())?,
                command_line(self.executable.as_os_str(), &self.args)?,
            ),
            CommandSyntax::CmdInvocation { switches } => (
                wide(system_directory()?.join("cmd.exe").as_os_str())?,
                command::invocation_line(&self.executable, &self.args, switches, &self.cwd)?,
            ),
            CommandSyntax::CmdSource { switches, source } => {
                if !self.args.is_empty() {
                    return Err(invalid(
                        "cmd source cannot also supply an ordinary argument vector",
                    ));
                }
                (
                    wide(system_directory()?.join("cmd.exe").as_os_str())?,
                    command::source_line(switches, source)?,
                )
            }
        };
        if !matches!(self.syntax, CommandSyntax::Argv)
            && self
                .environment
                .iter()
                .any(|(_, value)| value.encode_wide().count() > 8191)
        {
            return Err(invalid(
                "cmd cannot preserve an inherited environment value longer than 8191 units",
            ));
        }
        let cwd = wide(self.cwd.as_os_str())?;
        let environment = environment(self.environment)?;
        let (stdin, input) = prepare_stdio(self.stdin, true).await?;
        let (stdout, output) = prepare_stdio(self.stdout, false).await?;
        let (stderr, error) = prepare_stdio(self.stderr, false).await?;
        let mut inherited = vec![input, output, error];
        inherited.extend(self.inherited);
        let handles: Vec<HANDLE> = inherited.iter().map(AsRawHandle::as_raw_handle).collect();
        for handle in &handles {
            // SAFETY: every handle is borrowed from a retained OwnedHandle. The
            // complete list is passed atomically, and all parent copies close.
            if unsafe { SetHandleInformation(*handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) }
                == 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        let job = match self.lifetime {
            Lifetime::OwnedJob => Some(create_job(false)?),
            #[cfg(feature = "test-support")]
            Lifetime::FixtureBreakawayJob => Some(create_job(true)?),
            Lifetime::TrustedSupervisor | Lifetime::IndependentService => None,
        };
        let attributes = Attributes::new(if job.is_some() { 2 } else { 1 })?;
        attributes.add(
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
            handles.as_ptr().cast(),
            mem::size_of_val(handles.as_slice()),
        )?;
        let job_handles = [job
            .as_ref()
            .map_or(ptr::null_mut(), AsRawHandle::as_raw_handle)];
        if job.is_some() {
            attributes.add(
                PROC_THREAD_ATTRIBUTE_JOB_LIST,
                job_handles.as_ptr().cast(),
                size_of::<HANDLE>(),
            )?;
        }
        // SAFETY: these Windows C structures permit zero-initialized optional
        // fields. Required size, handles and attribute pointer are set below.
        let mut startup: STARTUPINFOEXW = unsafe { mem::zeroed() };
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = handles[0];
        startup.StartupInfo.hStdOutput = handles[1];
        startup.StartupInfo.hStdError = handles[2];
        startup.lpAttributeList = attributes.pointer;
        let mut flags = EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT;
        if self.lifetime == Lifetime::IndependentService {
            flags |= CREATE_BREAKAWAY_FROM_JOB;
        }
        match self.console {
            Console::Inherit => (),
            Console::NewProcessGroup => flags |= CREATE_NEW_PROCESS_GROUP,
            Console::PrivateHidden => {
                flags |= CREATE_NEW_CONSOLE;
                startup.StartupInfo.dwFlags |= STARTF_USESHOWWINDOW;
                startup.StartupInfo.wShowWindow = SW_HIDE as u16;
            }
        }
        let create = |flags| {
            // CreateProcessW may modify its command-line buffer even when it
            // fails. Each attempt therefore receives a fresh copy and a fresh
            // output structure while all immutable attributes and handles stay
            // retained across the synchronous call.
            let mut command_line = command_line.clone();
            // SAFETY: output-only plain C structure; all buffers, attributes
            // and handles remain live. Job assignment happens inside creation,
            // before any child code; no suspended orphan interval.
            let mut process: PROCESS_INFORMATION = unsafe { mem::zeroed() };
            let created = unsafe {
                CreateProcessW(
                    executable.as_ptr(),
                    command_line.as_mut_ptr(),
                    ptr::null(),
                    ptr::null(),
                    1,
                    flags,
                    environment.as_ptr().cast(),
                    cwd.as_ptr(),
                    &startup.StartupInfo,
                    &mut process,
                )
            };
            if created == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(process)
            }
        };
        let process = match create(flags) {
            Ok(process) => process,
            Err(error)
                if self.lifetime == Lifetime::IndependentService
                    && error.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) =>
            {
                let mut in_job = 0;
                // SAFETY: the current-process pseudo handle is borrowed, null
                // requests membership in any Job, and in_job is writable.
                if unsafe { IsProcessInJob(GetCurrentProcess(), ptr::null_mut(), &mut in_job) } == 0
                {
                    return Err(io::Error::last_os_error());
                }
                if in_job == 0 {
                    return Err(error);
                }
                create(flags & !CREATE_BREAKAWAY_FROM_JOB)?
            }
            Err(error) => return Err(error),
        };
        // SAFETY: successful creation transfers both independent owned handles.
        let process_handle = unsafe { OwnedHandle::from_raw_handle(process.hProcess) };
        // SAFETY: same transfer; the primary thread handle is not needed.
        drop(unsafe { OwnedHandle::from_raw_handle(process.hThread) });
        // No await or fallible operation occurs after creation before the owner
        // is returned. Closing inherited parent endpoints permits genuine EOF.
        drop(inherited);
        Ok(NativeChild {
            process: process_handle,
            job,
            id: process.dwProcessId,
            status: None,
            console: self.console,
            stdin,
            stdout,
            stderr,
        })
    }
}

/// Retained process identity and optional owned tree. A timeout leaves ownership
/// intact. Drop initiates Job cleanup; only a successful wait proves completion.
pub struct NativeChild {
    process: OwnedHandle,
    job: Option<OwnedHandle>,
    id: u32,
    status: Option<ExitStatus>,
    console: Console,
    stdin: Option<Pipe>,
    stdout: Option<Pipe>,
    stderr: Option<Pipe>,
}

impl NativeChild {
    pub fn id(&self) -> u32 {
        self.id
    }

    /// A wait/query-only duplicate; it cannot confer arbitrary PID authority.
    pub fn duplicate_process_handle(&self) -> io::Result<OwnedHandle> {
        duplicate_process(self.process.as_raw_handle())
    }
    /// A diagnostics-only duplicate carrying `PROCESS_QUERY_LIMITED_INFORMATION`,
    /// for sampling CPU time and working set on an already-slow/timed-out path.
    /// That single right is sufficient for both `GetProcessTimes` and
    /// `K32GetProcessMemoryInfo`; it has no terminate, suspend, or write
    /// authority and confers no PID ownership.
    pub fn duplicate_diagnostic_handle(&self) -> io::Result<OwnedHandle> {
        duplicate_process_with_access(
            self.process.as_raw_handle(),
            PROCESS_QUERY_LIMITED_INFORMATION,
        )
    }
    /// Native fixture observation for a root inside an owned Job whose
    /// descendants intentionally remain active. The retained process handle,
    /// rather than its numeric ID, supplies the identity.
    #[cfg(feature = "test-support")]
    pub fn fixture_root_has_exited(&self) -> io::Result<bool> {
        Ok(!process_is_running(&self.process)?)
    }
    pub fn take_stdin(&mut self) -> Option<Pipe> {
        self.stdin.take()
    }
    pub fn take_stdout(&mut self) -> Option<Pipe> {
        self.stdout.take()
    }
    pub fn take_stderr(&mut self) -> Option<Pipe> {
        self.stderr.take()
    }

    pub(super) fn is_running(&self) -> io::Result<bool> {
        process_is_running(&self.process)
    }

    pub(super) fn identity(&self) -> io::Result<(u32, OwnedHandle)> {
        Ok((self.id, self.process.try_clone()?))
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if self.status.is_none() {
            if self.is_running()? {
                return Ok(None);
            }
            let mut code = 0;
            // SAFETY: retained process handle; code points to writable output.
            if unsafe { GetExitCodeProcess(self.process.as_raw_handle(), &mut code) } == 0 {
                return Err(io::Error::last_os_error());
            }
            self.status = Some(ExitStatus::from_raw(code));
        }
        if let Some(job) = &self.job
            && job_accounting(job)?.ActiveProcesses != 0
        {
            return Ok(None);
        }
        Ok(self.status)
    }

    /// A read-only, point-in-time description of this child and its owned Job
    /// for failure diagnostics. It neither caches status nor signals or waits.
    ///
    /// Job members are listed by numeric ID and, best effort, image name and
    /// resource sample through a query-only handle. An ID can be reused
    /// between listing and opening; a reopened process outside this Job is
    /// reported as such rather than named. The IDs are text only and confer
    /// no authority: termination remains with the retained Job handle.
    pub fn diagnostic_snapshot(&self) -> TreeSnapshot {
        let root = process_is_running(&self.process).and_then(|running| {
            if running {
                return Ok(RootObservation::Running);
            }
            let mut code = 0;
            // SAFETY: retained process handle; code points to writable output.
            if unsafe { GetExitCodeProcess(self.process.as_raw_handle(), &mut code) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(RootObservation::Exited(code))
        });
        TreeSnapshot {
            root_id: self.id,
            root,
            // The creation handle carries query rights; this is a final sample
            // regardless of how short the caller's deadline was.
            root_sample: sample_process(&self.process),
            job: self.job.as_ref().map(job_snapshot),
        }
    }

    pub async fn wait(&mut self, timeout: Duration) -> io::Result<ExitStatus> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "native process tree did not become quiescent",
                ));
            }
            tokio::time::sleep_until(
                deadline.min(tokio::time::Instant::now() + Duration::from_millis(10)),
            )
            .await;
        }
    }

    /// Request termination using retained ownership, then call wait to prove it.
    pub fn terminate(&mut self) -> io::Result<()> {
        if self.try_wait()?.is_some() {
            return Ok(());
        }
        // SAFETY: each branch uses a retained owned handle, never a reopened PID.
        let result = unsafe {
            match &self.job {
                Some(job) => TerminateJobObject(job.as_raw_handle(), 1),
                None => TerminateProcess(self.process.as_raw_handle(), 1),
            }
        };
        if result == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Target CTRL_BREAK only at a retained live child created as a new group.
    pub fn interrupt(&mut self) -> io::Result<()> {
        if self.console != Console::NewProcessGroup {
            return Err(invalid(
                "CTRL_BREAK requires an explicitly created console process group",
            ));
        }
        if !self.is_running()? {
            return Ok(());
        }
        // SAFETY: a live owned process handle retains this process ID; the
        // group was established at creation and cannot name an unrelated PID.
        if unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, self.id) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

/// Root state observed without reaping or caching it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RootObservation {
    Running,
    Exited(u32),
}

/// See [`NativeChild::diagnostic_snapshot`]. Diagnostics only: never
/// consulted for control flow, only appended to failure text.
#[derive(Debug)]
pub struct TreeSnapshot {
    pub root_id: u32,
    pub root: io::Result<RootObservation>,
    pub root_sample: io::Result<ProcessSample>,
    /// `None` when this child has no owned Job.
    pub job: Option<io::Result<JobSnapshot>>,
}

/// Owned Job accounting and its listed members at one instant.
#[derive(Debug)]
pub struct JobSnapshot {
    pub active_processes: u32,
    pub total_processes: u32,
    pub terminated_processes: u32,
    /// Members the Job reported; more than `members.len()` when capped.
    pub assigned_processes: u32,
    pub members: Vec<JobMember>,
}

#[derive(Debug)]
pub struct JobMember {
    pub id: u32,
    /// Executable file name, without its directory.
    pub image: io::Result<OsString>,
    pub sample: io::Result<ProcessSample>,
}

/// Bound on listed members; further members are only counted.
const JOB_MEMBER_LIMIT: usize = 256;

impl fmt::Display for TreeSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "root pid={} ", self.root_id)?;
        match &self.root {
            Ok(RootObservation::Running) => write!(formatter, "state=running")?,
            Ok(RootObservation::Exited(code)) => write!(formatter, "state=exited({code})")?,
            Err(error) => write!(formatter, "state_error={error}")?,
        }
        write_sample(formatter, &self.root_sample)?;
        match &self.job {
            None => write!(formatter, "; job=none"),
            Some(Err(error)) => write!(formatter, "; job_error={error}"),
            Some(Ok(job)) => {
                write!(
                    formatter,
                    "; job active={} total={} terminated={} listed={}/{} members=[",
                    job.active_processes,
                    job.total_processes,
                    job.terminated_processes,
                    job.members.len(),
                    job.assigned_processes
                )?;
                for (index, member) in job.members.iter().enumerate() {
                    if index > 0 {
                        write!(formatter, "; ")?;
                    }
                    write!(formatter, "pid={} ", member.id)?;
                    match &member.image {
                        Ok(image) => write!(formatter, "image={}", image.to_string_lossy())?,
                        Err(error) => write!(formatter, "image_error={error}")?,
                    }
                    write_sample(formatter, &member.sample)?;
                }
                write!(formatter, "]")
            }
        }
    }
}

fn write_sample(
    formatter: &mut fmt::Formatter<'_>,
    sample: &io::Result<ProcessSample>,
) -> fmt::Result {
    match sample {
        Ok(sample) => write!(
            formatter,
            " cpu={}ms working_set={}B",
            (sample.kernel_time + sample.user_time).as_millis(),
            sample.working_set_bytes
        ),
        Err(error) => write!(formatter, " sample_error={error}"),
    }
}

fn job_accounting(job: &OwnedHandle) -> io::Result<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION> {
    // SAFETY: zeroable output buffer with the exact declared size/class.
    let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { mem::zeroed() };
    if unsafe {
        QueryInformationJobObject(
            job.as_raw_handle(),
            JobObjectBasicAccountingInformation,
            (&mut accounting as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
            size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(accounting)
}

fn job_snapshot(job: &OwnedHandle) -> io::Result<JobSnapshot> {
    let accounting = job_accounting(job)?;
    let (assigned_processes, ids) = job_process_ids(job)?;
    let mut image = vec![0u16; 32 * 1024];
    let members = ids
        .into_iter()
        .map(|id| {
            let (image, sample) = match open_member(job, id) {
                Ok(process) => (image_name(&process, &mut image), sample_process(&process)),
                Err(error) => (Err(error), Err(io::Error::other("member not opened"))),
            };
            JobMember { id, image, sample }
        })
        .collect();
    Ok(JobSnapshot {
        active_processes: accounting.ActiveProcesses,
        total_processes: accounting.TotalProcesses,
        terminated_processes: accounting.TotalTerminatedProcesses,
        assigned_processes,
        members,
    })
}

/// The variable-length ID list grows once to the reported member count and
/// is otherwise capped; a still-longer list is returned truncated.
fn job_process_ids(job: &OwnedHandle) -> io::Result<(u32, Vec<u32>)> {
    let header = mem::offset_of!(JOBOBJECT_BASIC_PROCESS_ID_LIST, ProcessIdList);
    let header_words = header.div_ceil(size_of::<usize>());
    let mut capacity = 64usize;
    for attempt in 0..2 {
        let mut buffer = vec![0usize; header_words + capacity];
        // SAFETY: the usize buffer is suitably aligned for the list header and
        // its array, and the declared length is exactly its size in bytes.
        let result = unsafe {
            QueryInformationJobObject(
                job.as_raw_handle(),
                JobObjectBasicProcessIdList,
                buffer.as_mut_ptr().cast(),
                mem::size_of_val(buffer.as_slice()) as u32,
                ptr::null_mut(),
            )
        };
        let truncated = if result == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_MORE_DATA as i32) {
                return Err(error);
            }
            true
        } else {
            false
        };
        // SAFETY: the buffer begins with the list's two u32 counters, which
        // the query initializes on success and on ERROR_MORE_DATA.
        let (assigned, listed) = unsafe {
            let counters = buffer.as_ptr().cast::<u32>();
            (*counters, *counters.add(1))
        };
        let wanted = (assigned as usize).saturating_add(16).min(JOB_MEMBER_LIMIT);
        if truncated && attempt == 0 && wanted > capacity {
            capacity = wanted;
            continue;
        }
        let listed = (listed as usize).min(capacity);
        let ids = buffer[header_words..header_words + listed]
            .iter()
            .map(|&id| id as u32)
            .collect();
        return Ok((assigned, ids));
    }
    Err(io::Error::other(
        "Job process list changed during its bounded query",
    ))
}

/// Open a listed ID with query rights only, and confirm the opened process is
/// still in this Job, so a reused ID cannot be named as a member.
fn open_member(job: &OwnedHandle, id: u32) -> io::Result<OwnedHandle> {
    // SAFETY: query-only access to a listed ID; a null result is an error.
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, id) };
    if raw.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful OpenProcess transfers one owned handle, closed on drop.
    let process = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut member = 0;
    // SAFETY: both handles are retained for the call; member is writable.
    if unsafe { IsProcessInJob(process.as_raw_handle(), job.as_raw_handle(), &mut member) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if member == 0 {
        return Err(io::Error::other("ID now names a process outside this Job"));
    }
    Ok(process)
}

fn image_name(process: &OwnedHandle, buffer: &mut [u16]) -> io::Result<OsString> {
    let mut length = buffer.len() as u32;
    // SAFETY: the retained query handle and writable buffer outlive the call;
    // length carries the buffer capacity in UTF-16 units and receives the result.
    if unsafe {
        QueryFullProcessImageNameW(
            process.as_raw_handle(),
            PROCESS_NAME_WIN32,
            buffer.as_mut_ptr(),
            &mut length,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let path = PathBuf::from(OsString::from_wide(&buffer[..length as usize]));
    Ok(path
        .file_name()
        .map_or_else(|| path.clone().into_os_string(), OsStr::to_os_string))
}

/// Retain a real handle to this process, rather than inheriting a pseudo-handle.
pub fn current_process_handle() -> io::Result<OwnedHandle> {
    // SAFETY: the process pseudo-handle is used only as an input to duplication.
    duplicate_process(unsafe { GetCurrentProcess() })
}

/// Duplicate an inherited token without taking ownership of or closing that
/// token. Verify the resulting object's type and expected process identity.
/// The original inherited handle remains with its creator/OS until process exit.
pub fn duplicate_inherited_process_handle(
    value: usize,
    expected_pid: u32,
) -> io::Result<OwnedHandle> {
    if value == 0 || value == usize::MAX || expected_pid == 0 {
        return Err(invalid("invalid inherited process handle"));
    }
    let handle = duplicate_process(value as HANDLE)?;
    // SAFETY: the duplicate is a retained owned kernel handle. GetProcessId
    // validates its object type; no supplied process ID is opened or terminated.
    let pid = unsafe { GetProcessId(handle.as_raw_handle()) };
    if pid == 0 || pid != expected_pid {
        return Err(invalid(
            "inherited process handle does not match its expected process",
        ));
    }
    Ok(handle)
}

fn duplicate_process(handle: HANDLE) -> io::Result<OwnedHandle> {
    duplicate_process_with_access(
        handle,
        PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
    )
}

fn duplicate_process_with_access(handle: HANDLE, access: u32) -> io::Result<OwnedHandle> {
    let mut duplicate = ptr::null_mut();
    // SAFETY: DuplicateHandle validates an opaque source handle and writes one
    // new handle to our output; failure never transfers ownership of the input.
    let result = unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            handle,
            GetCurrentProcess(),
            &mut duplicate,
            access,
            0,
            0,
        )
    };
    if result == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful duplication returned a unique owned handle.
    Ok(unsafe { OwnedHandle::from_raw_handle(duplicate) })
}

/// A cheap point-in-time process resource sample. Diagnostics only: never
/// consulted for control flow, only appended to failure text.
#[derive(Clone, Copy, Debug)]
pub struct ProcessSample {
    pub kernel_time: Duration,
    pub user_time: Duration,
    pub working_set_bytes: u64,
}

/// Sample CPU time (kernel + user) and current working set for a retained
/// process handle. The handle needs only `PROCESS_QUERY_LIMITED_INFORMATION`
/// (see [`NativeChild::duplicate_diagnostic_handle`]).
pub fn sample_process(handle: &OwnedHandle) -> io::Result<ProcessSample> {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: handle is a retained, still-open process handle with sufficient
    // query rights; all four output pointers are valid FILETIME locations for
    // the duration of this call.
    if unsafe {
        GetProcessTimes(
            handle.as_raw_handle(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    // SAFETY: `cb` is set to this exact struct's size before the call, as
    // K32GetProcessMemoryInfo requires; PROCESS_QUERY_LIMITED_INFORMATION is
    // sufficient for this call, so the handle need not carry PROCESS_VM_READ.
    // K32GetProcessMemoryInfo is the kernel32-exported form of psapi's
    // GetProcessMemoryInfo (identical signature, available since Windows 7);
    // using it keeps this crate's shipping imports on kernel32 alone.
    if unsafe { K32GetProcessMemoryInfo(handle.as_raw_handle(), &mut counters, counters.cb) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(ProcessSample {
        kernel_time: filetime_to_duration(kernel),
        user_time: filetime_to_duration(user),
        working_set_bytes: counters.WorkingSetSize as u64,
    })
}

/// A process id bound to the creation time of the process that held it.
///
/// Diagnostics only: it confers no authority over that id, and nothing may
/// signal, wait on or terminate a process because its stamp matches. An id can
/// be reused once its process object is gone; the creation time tells a reused
/// id apart from the original process.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessStamp {
    pub id: u32,
    /// Creation time in FILETIME ticks (100 ns intervals since 1601-01-01 UTC).
    pub created: u64,
}

impl NativeChild {
    /// This child's id and creation time, read through the retained handle.
    pub fn stamp(&self) -> io::Result<ProcessStamp> {
        process_stamp(&self.process)
    }
}

/// The id and creation time of the process a retained handle refers to. The
/// handle needs only `PROCESS_QUERY_LIMITED_INFORMATION`.
pub fn process_stamp(handle: &OwnedHandle) -> io::Result<ProcessStamp> {
    // SAFETY: the owned handle is retained for this call; a non-process or a
    // handle without query rights fails with zero.
    let id = unsafe { GetProcessId(handle.as_raw_handle()) };
    if id == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: the retained handle carries query rights; all four outputs are
    // valid FILETIME locations for the duration of this call.
    if unsafe {
        GetProcessTimes(
            handle.as_raw_handle(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(ProcessStamp {
        id,
        created: (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime),
    })
}

/// Whether a process object with exactly this stamp can still be opened.
///
/// A process object, and with it the image section of its executable, lives
/// until the last handle to it closes, including after the process has exited.
/// `true` therefore means that some handle, in this process or another one,
/// still keeps that object (and its image) alive. `false` means no object with
/// this id exists any more, or the id now names a later process. Any other
/// failure to open or query it, such as access denied, is returned as an error
/// rather than guessed. Diagnostics only: the result never authorizes acting
/// on the process.
pub fn process_object_retained(stamp: ProcessStamp) -> io::Result<bool> {
    // SAFETY: query-only access by id; a null result is an error, never a handle.
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, stamp.id) };
    if raw.is_null() {
        let error = io::Error::last_os_error();
        // OpenProcess reports an id that names no process object as an
        // invalid parameter.
        return if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
            Ok(false)
        } else {
            Err(error)
        };
    }
    // SAFETY: successful OpenProcess transfers one owned handle, closed on drop.
    let process = unsafe { OwnedHandle::from_raw_handle(raw) };
    Ok(process_stamp(&process)?.created == stamp.created)
}

fn filetime_to_duration(value: FILETIME) -> Duration {
    let ticks = (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime);
    // FILETIME ticks are 100-nanosecond intervals.
    Duration::from_nanos(ticks.saturating_mul(100))
}

/// Wait on a retained process object, bounded without reopening its numeric PID.
pub async fn wait_process_handle(handle: &OwnedHandle, timeout: Duration) -> io::Result<()> {
    // SAFETY: the owned handle is retained for this call; reject a nonprocess.
    if unsafe { GetProcessId(handle.as_raw_handle()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let deadline = tokio::time::Instant::now() + timeout;
    while process_is_running(handle)? {
        if tokio::time::Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "retained process has not exited",
            ));
        }
        tokio::time::sleep_until(
            deadline.min(tokio::time::Instant::now() + Duration::from_millis(10)),
        )
        .await;
    }
    Ok(())
}

async fn prepare_stdio(stdio: Stdio, child_reads: bool) -> io::Result<(Option<Pipe>, OwnedHandle)> {
    match stdio {
        Stdio::Pipe => pipe::stdio(child_reads)
            .await
            .map(|(parent, child)| (Some(parent), child)),
        Stdio::Handle(handle) => Ok((None, handle)),
        Stdio::Null => {
            let name = wide(OsStr::new("NUL"))?;
            // SAFETY: terminated literal name, no inherited security pointer.
            let raw = unsafe {
                CreateFileW(
                    name.as_ptr(),
                    if child_reads {
                        GENERIC_READ
                    } else {
                        GENERIC_WRITE
                    },
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    ptr::null(),
                    OPEN_EXISTING,
                    0,
                    ptr::null_mut(),
                )
            };
            if raw == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: successful CreateFile transfers a unique owned handle.
            Ok((None, unsafe { OwnedHandle::from_raw_handle(raw) }))
        }
    }
}

pub(super) fn process_is_running(process: &OwnedHandle) -> io::Result<bool> {
    // SAFETY: process handle remains owned for the entire nonblocking wait.
    match unsafe { WaitForSingleObject(process.as_raw_handle(), 0) } {
        WAIT_TIMEOUT => Ok(true),
        WAIT_OBJECT_0 => Ok(false),
        _ => Err(io::Error::last_os_error()),
    }
}

fn create_job(allow_breakaway: bool) -> io::Result<OwnedHandle> {
    // SAFETY: unnamed Job, null security attributes makes handle non-inheritable.
    let raw = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    if raw.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful native creation transfers one owned handle.
    let job = unsafe { OwnedHandle::from_raw_handle(raw) };
    // SAFETY: zeroed optional job settings, exact class/size supplied below.
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    if allow_breakaway {
        limits.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_BREAKAWAY_OK;
    }
    if unsafe {
        SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(job)
}

/// HeapAlloc supplies native alignment for this opaque variable-sized API.
struct Attributes {
    pointer: LPPROC_THREAD_ATTRIBUTE_LIST,
}
impl Attributes {
    fn new(count: u32) -> io::Result<Self> {
        let mut size = 0;
        // SAFETY: documented sizing call; size is writable, null is intentional.
        unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), count, 0, &mut size) };
        if size == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: native heap allocation owns an aligned buffer of queried size.
        let pointer = unsafe { HeapAlloc(GetProcessHeap(), 0, size) };
        if pointer.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                "native process attribute allocation failed",
            ));
        }
        // SAFETY: allocation covers queried size with native alignment.
        if unsafe { InitializeProcThreadAttributeList(pointer, count, 0, &mut size) } == 0 {
            let error = io::Error::last_os_error();
            // SAFETY: this allocation has not yet become a live attribute list.
            unsafe { HeapFree(GetProcessHeap(), 0, pointer) };
            return Err(error);
        }
        Ok(Self { pointer })
    }
    fn add(&self, attribute: u32, value: *const c_void, size: usize) -> io::Result<()> {
        // SAFETY: all callers provide retained typed slices through CreateProcess;
        // this private method never exposes untracked attribute storage publicly.
        if unsafe {
            UpdateProcThreadAttribute(
                self.pointer,
                0,
                attribute as usize,
                value,
                size,
                ptr::null_mut(),
                ptr::null(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        // SAFETY: list initialized once, allocation owned uniquely by self.
        unsafe {
            DeleteProcThreadAttributeList(self.pointer);
            HeapFree(GetProcessHeap(), 0, self.pointer);
        }
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
fn wide(value: &OsStr) -> io::Result<Vec<u16>> {
    let mut result: Vec<u16> = value.encode_wide().collect();
    if result.contains(&0) {
        return Err(invalid("native strings must not contain NUL"));
    }
    result.push(0);
    Ok(result)
}

fn command_line(executable: &OsStr, args: &[OsString]) -> io::Result<Vec<u16>> {
    let mut result = Vec::new();
    for argument in std::iter::once(executable).chain(args.iter().map(OsString::as_os_str)) {
        if !result.is_empty() {
            result.push(b' ' as u16);
        }
        let units = wide(argument)?;
        let units = &units[..units.len() - 1];
        // Ordinary Windows argv quoting: quote every argument; 2n backslashes
        // before a closing quote, 2n+1 before a literal quote, n otherwise.
        result.push(b'"' as u16);
        let mut slashes = 0;
        for &unit in units {
            if unit == b'\\' as u16 {
                slashes += 1;
                continue;
            }
            result.extend(std::iter::repeat_n(
                b'\\' as u16,
                if unit == b'"' as u16 {
                    slashes * 2 + 1
                } else {
                    slashes
                },
            ));
            slashes = 0;
            result.push(unit);
        }
        result.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
        result.push(b'"' as u16);
    }
    result.push(0);
    if result.len() > 32767 {
        return Err(invalid("native command line exceeds Windows limit"));
    }
    Ok(result)
}

fn environment(values: Vec<(OsString, OsString)>) -> io::Result<Vec<u16>> {
    let mut entries = Vec::with_capacity(values.len());
    for (key, value) in values {
        let key = wide(&key)?;
        let value = wide(&value)?;
        if key.len() <= 1
            || (key[..key.len() - 1].contains(&(b'=' as u16))
                && !(key.len() == 4
                    && key[0] == b'=' as u16
                    && ((b'A' as u16..=b'Z' as u16).contains(&key[1])
                        || (b'a' as u16..=b'z' as u16).contains(&key[1]))
                    && key[2] == b':' as u16))
            || key.len() > 32767
            || value.len() > 32767
        {
            return Err(invalid("invalid native environment entry"));
        }
        entries.push((key, value));
    }
    entries.sort_by(|(left, _), (right, _)| compare_keys(left, right));
    if entries
        .windows(2)
        .any(|pair| compare_keys(&pair[0].0, &pair[1].0) == Ordering::Equal)
    {
        return Err(invalid(
            "duplicate case-insensitive native environment keys",
        ));
    }
    let mut block = Vec::new();
    for (key, value) in entries {
        block.extend_from_slice(&key[..key.len() - 1]);
        block.push(b'=' as u16);
        block.extend(value);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    Ok(block)
}
fn compare_keys(left: &[u16], right: &[u16]) -> Ordering {
    // SAFETY: validated length-bounded terminated UTF-16 entries. CompareString-
    // Ordinal handles Windows Unicode case folding without a locale dependency.
    match unsafe {
        CompareStringOrdinal(
            left.as_ptr(),
            (left.len() - 1) as i32,
            right.as_ptr(),
            (right.len() - 1) as i32,
            1,
        )
    } {
        CSTR_LESS_THAN => Ordering::Less,
        CSTR_EQUAL => Ordering::Equal,
        _ => Ordering::Greater,
    }
}
