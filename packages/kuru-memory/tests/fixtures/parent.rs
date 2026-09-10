//! A compiled fixture with the real memory-owner lifetime and private handshake.
#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
#[path = "parent/windows.rs"]
mod windows;

#[cfg(windows)]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    windows::run().await
}
