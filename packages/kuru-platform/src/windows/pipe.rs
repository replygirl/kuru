//! Private event-backed overlapped pipes, with explicit cancellation and close.
//! A queued write is accepted into an owned buffer; flush observes its native
//! completion. Closing cancels undelivered I/O and waits before freeing buffers.

use super::{
    process::{NativeChild, process_is_running},
    security::PrivateSecurity,
};
use std::{
    cell::UnsafeCell,
    ffi::{OsStr, OsString},
    future::{Future, poll_fn},
    io, mem,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    pin::Pin,
    ptr,
    task::{Context, Poll},
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use windows_sys::Win32::{
    Foundation::{
        ERROR_BROKEN_PIPE, ERROR_IO_INCOMPLETE, ERROR_IO_PENDING, ERROR_NOT_FOUND, ERROR_PIPE_BUSY,
        ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
    },
    Storage::FileSystem::{
        CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, OPEN_EXISTING,
        PIPE_ACCESS_DUPLEX, PIPE_ACCESS_INBOUND, PIPE_ACCESS_OUTBOUND, ReadFile,
        SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, WriteFile,
    },
    System::{
        IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
        Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, PIPE_READMODE_BYTE,
            PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
        },
        Threading::CreateEventW,
    },
};

const PREFIX: &str = r"\\.\pipe\kuru-";
const POLL_INTERVAL: Duration = Duration::from_millis(5);
const MAX_WRITE: usize = 16 * 1024 * 1024;

fn address() -> OsString {
    format!("{PREFIX}{}", uuid::Uuid::new_v4()).into()
}
fn closed() -> io::Error {
    io::Error::new(
        io::ErrorKind::BrokenPipe,
        "native pipe is closing or closed",
    )
}
fn check_address(address: &OsStr) -> io::Result<()> {
    let value = address.to_string_lossy();
    let Some(name) = value.strip_prefix(PREFIX) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected a local Kuru pipe address",
        ));
    };
    if name.is_empty()
        || name.len() > 128
        || name.chars().any(|c| !c.is_ascii_alphanumeric() && c != '-')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid local pipe name",
        ));
    }
    Ok(())
}
fn wide(address: &OsStr) -> Vec<u16> {
    address.encode_wide().chain(Some(0)).collect()
}

#[derive(Clone, Copy)]
enum Kind {
    Read,
    Write,
    Connect,
}

/// Kernel references are confined to this owned, heap-stable allocation. No
/// buffer/OVERLAPPED is released until GetOverlappedResult observes completion.
struct Operation {
    overlapped: Box<UnsafeCell<OVERLAPPED>>,
    _event: OwnedHandle,
    buffer: Vec<u8>,
    offset: usize,
    result: Option<Result<usize, i32>>,
}
// SAFETY: the allocation and buffer remain at stable heap addresses when moved.
// Rust only accesses their contents after native completion. Methods require
// exclusive access; no shared callback or Rust reference aliases kernel writes.
unsafe impl Send for Operation {}

impl Operation {
    fn start(handle: HANDLE, kind: Kind, mut buffer: Vec<u8>, offset: usize) -> io::Result<Self> {
        // SAFETY: unnamed, non-inheritable manual-reset event.
        let event = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
        if event.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: successful native creation transfers one owned handle.
        let event = unsafe { OwnedHandle::from_raw_handle(event) };
        // SAFETY: OVERLAPPED optional fields are zeroable; retained event is set.
        let mut native: OVERLAPPED = unsafe { mem::zeroed() };
        native.hEvent = event.as_raw_handle();
        let overlapped = Box::new(UnsafeCell::new(native));
        // SAFETY: handle is retained by Resources; heap allocations and event
        // remain owned through completion. Buffers never resize while pending.
        let status = unsafe {
            match kind {
                Kind::Connect => ConnectNamedPipe(handle, overlapped.get()),
                Kind::Read => ReadFile(
                    handle,
                    buffer.as_mut_ptr(),
                    buffer.len() as u32,
                    ptr::null_mut(),
                    overlapped.get(),
                ),
                Kind::Write => WriteFile(
                    handle,
                    buffer[offset..].as_ptr(),
                    (buffer.len() - offset) as u32,
                    ptr::null_mut(),
                    overlapped.get(),
                ),
            }
        };
        let result = if status != 0 {
            None
        } else {
            let code = io::Error::last_os_error().raw_os_error().unwrap_or(1);
            if matches!(kind, Kind::Connect) && code == ERROR_PIPE_CONNECTED as i32 {
                Some(Ok(0))
            } else if code == ERROR_IO_PENDING as i32 {
                None
            } else {
                Some(Err(code))
            }
        };
        Ok(Self {
            overlapped,
            _event: event,
            buffer,
            offset,
            result,
        })
    }

    fn completion(&mut self, handle: HANDLE, wait: bool) -> Option<Result<usize, i32>> {
        if self.result.is_some() {
            return self.result;
        }
        let mut bytes = 0;
        // SAFETY: matching handle/OVERLAPPED are retained; writable output is
        // local. Kernel completion is the only authority to retire its buffers.
        if unsafe {
            GetOverlappedResult(handle, self.overlapped.get(), &mut bytes, i32::from(wait))
        } != 0
        {
            self.result = Some(Ok(bytes as usize));
        } else {
            let error = io::Error::last_os_error().raw_os_error().unwrap_or(1);
            if error != ERROR_IO_INCOMPLETE as i32 {
                self.result = Some(Err(error));
            }
        }
        self.result
    }
}

struct Resources {
    handle: OwnedHandle,
    read: Option<Operation>,
    write: Option<Operation>,
    connect: Option<Operation>,
}
impl Resources {
    fn raw(&self) -> HANDLE {
        self.handle.as_raw_handle()
    }
    fn cancel(&self) -> io::Result<()> {
        // SAFETY: owned handle; null means all this handle's outstanding I/O.
        if unsafe { CancelIoEx(self.raw(), ptr::null()) } == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_NOT_FOUND as i32) {
                return Err(error);
            }
        }
        Ok(())
    }
    fn completed(&mut self, wait: bool) -> bool {
        let handle = self.raw();
        let mut complete = true;
        for operation in [&mut self.read, &mut self.write, &mut self.connect]
            .into_iter()
            .flatten()
        {
            complete &= operation.completion(handle, wait).is_some();
        }
        complete
    }
}

/// Cancellable owned local byte channel. Writes queue at most 16 MiB per call;
/// `flush` waits for their actual native completion. `close` cancels undelivered
/// bytes and retains all ownership on timeout. Drop initiates cancellation but
/// never claims quiescence; a cleanup owner keeps buffers until the kernel ends.
pub struct Pipe {
    resources: Option<Resources>,
    closing: bool,
    readable: bool,
    writable: bool,
    ready_read: Vec<u8>,
    read_offset: usize,
    read_timer: Option<Pin<Box<tokio::time::Sleep>>>,
    write_timer: Option<Pin<Box<tokio::time::Sleep>>>,
    control_timer: Option<Pin<Box<tokio::time::Sleep>>>,
}
impl Pipe {
    fn new(handle: OwnedHandle, readable: bool, writable: bool) -> Self {
        Self {
            resources: Some(Resources {
                handle,
                read: None,
                write: None,
                connect: None,
            }),
            closing: false,
            readable,
            writable,
            ready_read: Vec::new(),
            read_offset: 0,
            read_timer: None,
            write_timer: None,
            control_timer: None,
        }
    }
    fn raw(&self) -> io::Result<HANDLE> {
        self.resources
            .as_ref()
            .map(Resources::raw)
            .ok_or_else(closed)
    }
    fn pending(&mut self, cx: &mut Context<'_>, kind: Kind) -> Poll<io::Result<()>> {
        // Split read/write tasks must retain independent wakers. Sharing one
        // Sleep lets the last writer erase the pending reader's wakeup.
        let slot = match kind {
            Kind::Read => &mut self.read_timer,
            Kind::Write => &mut self.write_timer,
            Kind::Connect => &mut self.control_timer,
        };
        let timer = slot.get_or_insert_with(|| Box::pin(tokio::time::sleep(POLL_INTERVAL)));
        if timer.as_mut().poll(cx).is_ready() {
            *slot = None;
            cx.waker().wake_by_ref();
        }
        Poll::Pending
    }
    fn begin_close(&mut self) -> io::Result<()> {
        if !self.closing {
            if let Some(resources) = &self.resources {
                resources.cancel()?;
            }
            self.closing = true;
        }
        Ok(())
    }
    fn poll_close(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.begin_close()?;
        if self
            .resources
            .as_mut()
            .is_some_and(|resources| !resources.completed(false))
        {
            return self.pending(cx, Kind::Connect);
        }
        self.resources = None;
        self.ready_read.clear();
        Poll::Ready(Ok(()))
    }
    pub async fn close(&mut self, timeout: Duration) -> io::Result<()> {
        tokio::time::timeout(timeout, poll_fn(|cx| self.poll_close(cx)))
            .await
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    "native pipe I/O did not finish cancellation",
                )
            })?
    }
    pub fn is_closed(&self) -> bool {
        self.resources.is_none()
    }

    async fn accept_connection(&mut self, timeout: Duration) -> io::Result<()> {
        let resources = self.resources.as_mut().ok_or_else(closed)?;
        if resources.connect.is_none() {
            resources.connect = Some(Operation::start(
                resources.raw(),
                Kind::Connect,
                Vec::new(),
                0,
            )?);
        }
        tokio::time::timeout(
            timeout,
            poll_fn(|cx| {
                let resources = self.resources.as_mut().ok_or_else(closed)?;
                let handle = resources.raw();
                let result = resources
                    .connect
                    .as_mut()
                    .expect("owned connect")
                    .completion(handle, false);
                match result {
                    Some(result) => {
                        resources.connect = None;
                        Poll::Ready(result.map(|_| ()).map_err(io::Error::from_raw_os_error))
                    }
                    None => self.pending(cx, Kind::Connect),
                }
            }),
        )
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "private pipe accept timed out"))?
    }
    fn poll_flush_write(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let resources = self.resources.as_mut().ok_or_else(closed)?;
        let handle = resources.raw();
        if let Some(operation) = &mut resources.write {
            let Some(result) = operation.completion(handle, false) else {
                return self.pending(cx, Kind::Write);
            };
            let mut operation = resources.write.take().expect("completed write");
            let bytes = result.map_err(io::Error::from_raw_os_error)?;
            operation.offset += bytes;
            if operation.offset < operation.buffer.len() {
                if bytes == 0 {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "native pipe write made no progress",
                    )));
                }
                resources.write = Some(Operation::start(
                    handle,
                    Kind::Write,
                    operation.buffer,
                    operation.offset,
                )?);
                return self.pending(cx, Kind::Write);
            }
        }
        Poll::Ready(Ok(()))
    }
}
impl AsyncRead for Pipe {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.closing || !self.readable {
            return Poll::Ready(Err(closed()));
        }
        if output.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        if self.ready_read.is_empty() {
            let resources = self.resources.as_mut().ok_or_else(closed)?;
            let handle = resources.raw();
            if resources.read.is_none() {
                resources.read = Some(Operation::start(
                    handle,
                    Kind::Read,
                    vec![0; output.remaining().min(65536)],
                    0,
                )?);
            }
            let Some(result) = resources
                .read
                .as_mut()
                .expect("owned read")
                .completion(handle, false)
            else {
                return self.pending(cx, Kind::Read);
            };
            let mut operation = resources.read.take().expect("completed read");
            match result {
                Ok(bytes) => {
                    operation.buffer.truncate(bytes);
                    self.ready_read = operation.buffer;
                    self.read_offset = 0;
                }
                Err(code) if code == ERROR_BROKEN_PIPE as i32 => return Poll::Ready(Ok(())),
                Err(code) => return Poll::Ready(Err(io::Error::from_raw_os_error(code))),
            }
        }
        let length = output
            .remaining()
            .min(self.ready_read.len() - self.read_offset);
        output.put_slice(&self.ready_read[self.read_offset..self.read_offset + length]);
        self.read_offset += length;
        if self.read_offset == self.ready_read.len() {
            self.ready_read.clear();
            self.read_offset = 0;
        }
        Poll::Ready(Ok(()))
    }
}
impl AsyncWrite for Pipe {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.closing || !self.writable {
            return Poll::Ready(Err(closed()));
        }
        match self.poll_flush_write(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
            Poll::Ready(Ok(())) => (),
        }
        let bytes = buffer.len().min(MAX_WRITE);
        if bytes == 0 {
            return Poll::Ready(Ok(0));
        }
        let resources = self.resources.as_mut().ok_or_else(closed)?;
        resources.write = Some(Operation::start(
            resources.raw(),
            Kind::Write,
            buffer[..bytes].to_vec(),
            0,
        )?);
        Poll::Ready(Ok(bytes))
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.closing {
            return Poll::Ready(Err(closed()));
        }
        self.poll_flush_write(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if !self.closing {
            match self.poll_flush_write(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(())) => (),
            }
        }
        self.poll_close(cx)
    }
}
impl Drop for Pipe {
    fn drop(&mut self) {
        let Some(mut resources) = self.resources.take() else {
            return;
        };
        let _ = resources.cancel();
        if resources.completed(false) {
            return;
        }
        // Keep native buffers alive even if OS cancellation has not completed.
        // This is a cancellation-only reaper, never a blocking Tokio I/O task.
        // If thread creation fails, preserve the allocation rather than free
        // kernel-referenced memory. The diagnostic never includes user payload.
        let pointer = Box::into_raw(Box::new(resources)) as usize;
        if std::thread::Builder::new()
            .name("kuru-pipe-reaper".into())
            .spawn(move || {
                // SAFETY: unique boxed owner transferred once; the failed-spawn path
                // deliberately retains it. The closure has sole Rust access.
                let mut resources = unsafe { Box::from_raw(pointer as *mut Resources) };
                if !resources.completed(true) {
                    // Even an unexpected incomplete result must not free a
                    // buffer still referenced by the kernel.
                    mem::forget(resources);
                    eprintln!("kuru: retained native pipe resources after incomplete cancellation");
                }
            })
            .is_err()
        {
            eprintln!(
                "kuru: retained cancelled native pipe resources after cleanup thread failure"
            );
        }
    }
}

fn server_with_instances(
    address: &OsStr,
    read: bool,
    write: bool,
    first: bool,
    instances: u32,
) -> io::Result<Pipe> {
    check_address(address)?;
    let descriptor = PrivateSecurity::new(GENERIC_READ | GENERIC_WRITE, false)?;
    let attributes = descriptor.attributes();
    let name = wide(address);
    let access = match (read, write) {
        (true, true) => PIPE_ACCESS_DUPLEX,
        (true, false) => PIPE_ACCESS_INBOUND,
        _ => PIPE_ACCESS_OUTBOUND,
    };
    // SAFETY: terminated name and descriptor remain live through creation;
    // resulting handle is local-only, first-instance, private and non-inheritable.
    let raw = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            access
                | FILE_FLAG_OVERLAPPED
                | if first {
                    FILE_FLAG_FIRST_PIPE_INSTANCE
                } else {
                    0
                },
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            instances,
            65536,
            65536,
            0,
            &attributes,
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful native creation transfers unique ownership.
    Ok(Pipe::new(
        unsafe { OwnedHandle::from_raw_handle(raw) },
        read,
        write,
    ))
}

fn server(address: &OsStr, read: bool, write: bool) -> io::Result<Pipe> {
    server_with_instances(address, read, write, true, 1)
}

/// Private local listener for a long-lived service. Unlike `PrivateListener`,
/// it accepts successive clients whose process identities are not known at
/// bind time. The caller must authenticate each connection at the protocol
/// layer; the DACL protects against other users, not a process with the same
/// user's access to the endpoint record.
pub struct PrivateServiceListener {
    address: OsString,
    next: Option<Pipe>,
}

impl PrivateServiceListener {
    pub fn bind() -> io::Result<Self> {
        Self::bind_at(&address())
    }

    pub fn bind_at(address: &OsStr) -> io::Result<Self> {
        Ok(Self {
            address: address.to_owned(),
            next: Some(server_with_instances(address, true, true, true, 255)?),
        })
    }

    pub fn address(&self) -> &OsStr {
        &self.address
    }

    pub async fn accept(&mut self, timeout: Duration) -> io::Result<Pipe> {
        if self.next.is_none() {
            self.next = Some(server_with_instances(
                &self.address,
                true,
                true,
                false,
                255,
            )?);
        }
        self.next
            .as_mut()
            .expect("private service pipe instance")
            .accept_connection(timeout)
            .await?;
        let connected = self.next.take().expect("connected service pipe instance");
        // A failure to create the next instance does not discard the accepted
        // client. The following accept retries creation with the same address.
        self.next = server_with_instances(&self.address, true, true, false, 255).ok();
        Ok(connected)
    }
}

pub struct PrivateListener {
    address: OsString,
    server: Pipe,
}
impl PrivateListener {
    pub fn bind() -> io::Result<Self> {
        Self::bind_at(&address())
    }
    pub fn bind_at(address: &OsStr) -> io::Result<Self> {
        Ok(Self {
            address: address.to_owned(),
            server: server(address, true, true)?,
        })
    }
    pub fn address(&self) -> &OsStr {
        &self.address
    }
    pub fn accept(
        mut self,
        expected: &NativeChild,
        timeout: Duration,
    ) -> impl Future<Output = io::Result<Pipe>> + Send + 'static + use<> {
        // Snapshot retained identity before suspension; no shared borrow of
        // unrelated child pipe state or unsafe Sync implementation is needed.
        let identity = expected.identity();
        async move {
            let (expected_id, process) = identity?;
            self.server.accept_connection(timeout).await?;
            let mut pid = 0;
            // SAFETY: connected owned handle and valid writable output.
            if unsafe { GetNamedPipeClientProcessId(self.server.raw()?, &mut pid) } == 0 {
                return Err(io::Error::last_os_error());
            }
            if pid != expected_id || !process_is_running(&process)? {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "private pipe connected by an unexpected process",
                ));
            }
            Ok(self.server)
        }
    }
}

fn open_client(
    address: &OsStr,
    read: bool,
    write: bool,
    overlapped: bool,
) -> io::Result<OwnedHandle> {
    let address = wide(address);
    // SECURITY_IDENTIFICATION explicitly prevents server impersonation; both
    // this overlapped rendezvous and parent-opened synchronous stdio use it.
    let flags = SECURITY_IDENTIFICATION
        | SECURITY_SQOS_PRESENT
        | if overlapped { FILE_FLAG_OVERLAPPED } else { 0 };
    let access = if read { GENERIC_READ } else { 0 } | if write { GENERIC_WRITE } else { 0 };
    // SAFETY: checked terminated name; returned handle is initially non-inheritable.
    let raw = unsafe {
        CreateFileW(
            address.as_ptr(),
            access,
            0,
            ptr::null(),
            OPEN_EXISTING,
            flags,
            ptr::null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful native creation transfers unique ownership.
    Ok(unsafe { OwnedHandle::from_raw_handle(raw) })
}

pub async fn connect(address: &OsStr, timeout: Duration) -> io::Result<Pipe> {
    check_address(address)?;
    tokio::time::timeout(timeout, async {
        loop {
            match open_client(address, true, true, true) {
                Ok(handle) => return Ok(Pipe::new(handle, true, true)),
                Err(error)
                    if error.kind() == io::ErrorKind::NotFound
                        || error.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) =>
                {
                    tokio::time::sleep(POLL_INTERVAL).await
                }
                Err(error) => return Err(error),
            }
        }
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "private pipe connect timed out"))?
}

/// The parent opens this synchronous endpoint for inheritance; its client PID
/// is therefore the parent, and the child-rendezvous identity check is invalid.
pub(super) async fn stdio(child_reads: bool) -> io::Result<(Pipe, OwnedHandle)> {
    let address = address();
    let mut parent = server(&address, !child_reads, child_reads)?;
    let child = open_client(&address, child_reads, !child_reads, false)?;
    parent.accept_connection(Duration::from_secs(5)).await?;
    Ok((parent, child))
}
