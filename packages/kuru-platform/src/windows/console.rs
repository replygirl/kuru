//! Retained console state. Raw-mode helpers may otherwise restore defaults
//! instead of the caller's actual modes. See Microsoft's Get/SetConsoleMode.

use super::process::{StandardStream, Stdio, inherited_stdio};
use std::{
    io,
    os::windows::io::{AsRawHandle, OwnedHandle},
};
use windows_sys::Win32::System::Console::{GetConsoleMode, SetConsoleMode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConsoleModes {
    pub input: u32,
    pub output: u32,
}

/// Both buffers are captured before any mode mutation. Handles retain the
/// original buffers even while a terminal changes its active screen buffer.
pub struct ConsoleModeGuard {
    input: OwnedHandle,
    output: OwnedHandle,
    original: ConsoleModes,
}

fn stream(channel: StandardStream) -> io::Result<OwnedHandle> {
    match inherited_stdio(channel)? {
        Stdio::Handle(handle) => Ok(handle),
        _ => Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "standard console handle is unavailable",
        )),
    }
}

fn mode(handle: &OwnedHandle) -> io::Result<u32> {
    let mut value = 0;
    // SAFETY: handle is retained, and value is a valid writable DWORD.
    if unsafe { GetConsoleMode(handle.as_raw_handle(), &mut value) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(value)
}

fn set(handle: &OwnedHandle, value: u32) -> io::Result<()> {
    // SAFETY: retained console handle; the OS validates supported mode bits.
    if unsafe { SetConsoleMode(handle.as_raw_handle(), value) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

impl ConsoleModeGuard {
    pub fn capture() -> io::Result<Self> {
        let input = stream(StandardStream::Input)?;
        let output = stream(StandardStream::Output)?;
        Self::from_handles(input, output)
    }

    fn from_handles(input: OwnedHandle, output: OwnedHandle) -> io::Result<Self> {
        let original = ConsoleModes {
            input: mode(&input)?,
            output: mode(&output)?,
        };
        Ok(Self {
            input,
            output,
            original,
        })
    }

    pub fn original(&self) -> ConsoleModes {
        self.original
    }

    pub fn current(&self) -> io::Result<ConsoleModes> {
        Ok(ConsoleModes {
            input: mode(&self.input)?,
            output: mode(&self.output)?,
        })
    }

    /// Explicit on every session: crossterm caches its ANSI support probe, so
    /// a subsequent session cannot rely on that probe re-enabling output mode.
    pub fn enable_virtual_terminal_output(&self) -> io::Result<()> {
        use windows_sys::Win32::System::Console::{
            ENABLE_PROCESSED_OUTPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
        };
        set(
            &self.output,
            mode(&self.output)? | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING,
        )
    }

    /// Attempt both restorations even if one fails; retain handles for retry.
    pub fn restore(&mut self) -> io::Result<()> {
        let input = set(&self.input, self.original.input);
        let output = set(&self.output, self.original.output);
        input?;
        output?;
        Ok(())
    }
}

impl Drop for ConsoleModeGuard {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

/// Test-facing native records: VT focus sequences do not exercise ReadConsoleInput.
#[cfg(feature = "test-support")]
pub fn inject_focus(focused: bool) -> io::Result<()> {
    queue_focus(&stream(StandardStream::Input)?, focused)
}

#[cfg(feature = "test-support")]
fn queue_focus(input: &OwnedHandle, focused: bool) -> io::Result<()> {
    use windows_sys::Win32::System::Console::{
        FOCUS_EVENT, FOCUS_EVENT_RECORD, INPUT_RECORD, INPUT_RECORD_0, WriteConsoleInputW,
    };
    let record = INPUT_RECORD {
        EventType: FOCUS_EVENT as u16,
        Event: INPUT_RECORD_0 {
            FocusEvent: FOCUS_EVENT_RECORD {
                bSetFocus: i32::from(focused),
            },
        },
    };
    let mut written = 0;
    // SAFETY: one fully initialized matching union variant and a valid count
    // pointer remain alive until the synchronous console call completes.
    if unsafe { WriteConsoleInputW(input.as_raw_handle(), &record, 1, &mut written) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if written != 1 {
        return Err(io::Error::other("focus record was not queued"));
    }
    Ok(())
}

/// Called only by the native platform fixture, in its own newly allocated
/// hidden console. It verifies actual restore/Drop and native record contents
/// without requiring the CI host itself to have terminal standard handles.
#[cfg(feature = "test-support")]
pub fn verify_private_console_fixture() -> io::Result<()> {
    use std::fs::OpenOptions;
    use windows_sys::Win32::System::Console::{
        ENABLE_INSERT_MODE, ENABLE_WRAP_AT_EOL_OUTPUT, FOCUS_EVENT, FlushConsoleInputBuffer,
        INPUT_RECORD, PeekConsoleInputW,
    };
    let input: OwnedHandle = OpenOptions::new()
        .read(true)
        .write(true)
        .open("CONIN$")?
        .into();
    let output: OwnedHandle = OpenOptions::new()
        .read(true)
        .write(true)
        .open("CONOUT$")?
        .into();
    let mut guard = ConsoleModeGuard::from_handles(input.try_clone()?, output.try_clone()?)?;
    let before = guard.original();
    guard.enable_virtual_terminal_output()?;
    set(&input, before.input ^ ENABLE_INSERT_MODE)?;
    set(&output, before.output ^ ENABLE_WRAP_AT_EOL_OUTPUT)?;
    if guard.current()? == before {
        return Err(io::Error::other("fixture modes did not change"));
    }
    guard.restore()?;
    if guard.current()? != before {
        return Err(io::Error::other("explicit console restore differs"));
    }
    set(&input, before.input ^ ENABLE_INSERT_MODE)?;
    drop(guard);
    if mode(&input)? != before.input {
        return Err(io::Error::other("Drop failed to restore console input"));
    }
    // SAFETY: this fixture owns its console input and has no concurrent reader.
    if unsafe { FlushConsoleInputBuffer(input.as_raw_handle()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    queue_focus(&input, false)?;
    queue_focus(&input, true)?;
    let mut records = [INPUT_RECORD::default(); 2];
    let mut count = 0;
    // SAFETY: two initialized writable records and count are retained; Peek is
    // nonblocking and cannot consume another application's input buffer.
    if unsafe { PeekConsoleInputW(input.as_raw_handle(), records.as_mut_ptr(), 2, &mut count) } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if count != 2
        || records
            .iter()
            .any(|record| record.EventType != FOCUS_EVENT as u16)
    {
        return Err(io::Error::other("native focus records are missing"));
    }
    // SAFETY: the preceding event tags establish both active union variants.
    if unsafe {
        records[0].Event.FocusEvent.bSetFocus != 0 || records[1].Event.FocusEvent.bSetFocus != 1
    } {
        return Err(io::Error::other("native focus state changed"));
    }
    Ok(())
}

/// Apply an unusual valid baseline in an isolated console fixture, so tests
/// distinguish exact restoration from crossterm's hard-coded default bits.
#[cfg(feature = "test-support")]
pub fn configure_test_baseline() -> io::Result<ConsoleModes> {
    use windows_sys::Win32::System::Console::{
        ENABLE_ECHO_INPUT, ENABLE_EXTENDED_FLAGS, ENABLE_INSERT_MODE, ENABLE_LINE_INPUT,
        ENABLE_PROCESSED_OUTPUT, ENABLE_QUICK_EDIT_MODE, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
    };
    let input = stream(StandardStream::Input)?;
    let output = stream(StandardStream::Output)?;
    let input_mode =
        (mode(&input)? | ENABLE_EXTENDED_FLAGS | ENABLE_LINE_INPUT | ENABLE_INSERT_MODE)
            & !(ENABLE_ECHO_INPUT | ENABLE_QUICK_EDIT_MODE);
    let output_mode =
        (mode(&output)? | ENABLE_PROCESSED_OUTPUT) & !ENABLE_VIRTUAL_TERMINAL_PROCESSING;
    set(&input, input_mode)?;
    set(&output, output_mode)?;
    Ok(ConsoleModes {
        input: input_mode,
        output: output_mode,
    })
}
