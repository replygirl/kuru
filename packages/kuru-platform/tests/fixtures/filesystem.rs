//! A native, immediate lock attempt in a separate process; no shell or retries.
use kuru_platform::fs::{Directory, NameRetention, Privacy};
use std::ffi::OsStr;
use std::fs::TryLockError;
use std::io;
use std::path::Path;

fn main() -> io::Result<()> {
    let arguments: Vec<_> = std::env::args_os().collect();
    if arguments.len() != 3 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected DIRECTORY LOCK_NAME",
        ));
    }
    let directory = Directory::open(
        Path::new(&arguments[1]),
        Privacy::OwnerOnly,
        NameRetention::Movable,
    )?;
    let lock = directory.lock_file(OsStr::new(&arguments[2]))?;
    match lock.try_lock() {
        Ok(()) => {
            directory.verify(OsStr::new(&arguments[2]), &lock)?;
            println!("acquired");
        }
        Err(TryLockError::WouldBlock) => println!("busy"),
        Err(TryLockError::Error(error)) => return Err(error),
    }
    Ok(())
}
