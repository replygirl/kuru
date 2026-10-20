use std::io::{self, Write};

use anyhow::{Context, Result};
use kuru_memory::MemoryStore;
use serde_json::{Value, json};

const STATE_SUFFIX: &str = "/notice/memory-storage";
const VERSION: u64 = 1;

/// App-owned, project-local presentation state. It intentionally never reaches
/// a provider, a harness transcript, or the runtime package.
#[derive(Clone)]
pub(crate) struct MemoryNotice {
    store: MemoryStore,
    key: String,
    directory: String,
    text: String,
}

/// Bounded escaped representation of the actual selected directory, including
/// spaces and control characters; shared by every memory disclosure surface.
pub(crate) fn display_directory(directory: &std::path::Path) -> String {
    format!("{directory:?}")
}

/// The persistent `/status` line naming where this project's memory lives.
pub(crate) fn status_line(directory: &str) -> String {
    format!("Memory: {directory} · kept until removed · kuru memory --help")
}

impl MemoryNotice {
    pub(crate) async fn pending(store: MemoryStore) -> Result<Option<Self>> {
        let status = store.status().await?;
        let key = format!("{}{}", status.project, STATE_SUFFIX);
        let value = store.get(&key).await?;
        let Some(value) = value else {
            return Ok(Some(Self::new(store, key, &status.directory)));
        };
        let version = value
            .get("version")
            .and_then(Value::as_u64)
            .context("stored first-run memory notice state has no numeric version")?;
        if version >= VERSION {
            return Ok(None);
        }
        Ok(Some(Self::new(store, key, &status.directory)))
    }

    fn new(store: MemoryStore, key: String, directory: &std::path::Path) -> Self {
        let directory = display_directory(directory);
        Self {
            store,
            key,
            text: format!(
                "Memory is ready at {directory}. Memory and chat do not expire automatically. See `kuru memory --help` to read, export, forget or purge it."
            ),
            directory,
        }
    }

    /// Interactive status-line indicator; the command reference lives in
    /// `kuru memory --help` rather than in the conversation.
    pub(crate) fn indicator(&self) -> String {
        format!(
            "Memory ready · kept until removed · /status · {}",
            self.directory
        )
    }

    /// Publish on stderr before committing the visible-version marker. Stdout,
    /// including machine-readable command output, is intentionally untouched.
    pub(crate) async fn announce(&self) -> Result<()> {
        let output = {
            let mut stderr = io::stderr().lock();
            write_notice(&mut stderr, &self.text)
        };
        output?;
        self.record().await
    }

    /// Call only after the TUI has completed the frame containing `text`.
    pub(crate) async fn record(&self) -> Result<()> {
        self.store
            .put(&self.key, &json!({ "version": VERSION }))
            .await
    }

    #[cfg(test)]
    async fn announce_to(&self, writer: &mut impl Write) -> Result<()> {
        write_notice(writer, &self.text)?;
        self.record().await
    }
}

fn write_notice(writer: &mut impl Write, text: &str) -> Result<()> {
    writeln!(writer, "{text}").context("write first-run memory notice")?;
    writer.flush().context("flush first-run memory notice")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FailingNoticeWriter {
        fail_flush: bool,
    }

    impl Write for FailingNoticeWriter {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            if self.fail_flush {
                Ok(1)
            } else {
                Err(io::Error::other("injected notice write failure"))
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::other("injected notice flush failure"))
        }
    }

    #[tokio::test]
    async fn state_is_pending_until_current_or_newer_and_rejects_malformed_values() {
        kuru_memory::test_support::closing(async {
            let store = {
                // Held across the supervisor and engine spawns; see `crate::spawn_gate`.
                let _gate = crate::spawn_gate::spawning().await;
                MemoryStore::temporary().await.unwrap()
            };
            let shutdown = store.clone();
            let notice = MemoryNotice::pending(store.clone()).await.unwrap().unwrap();
            let cleanup_directory = store.status().await.unwrap().directory;
            let directory = format!("{cleanup_directory:?}");
            assert!(notice.text.contains(&directory));
            assert!(notice.text.starts_with("Memory is ready at "));
            assert!(
                notice
                    .text
                    .contains("Memory and chat do not expire automatically.")
            );
            assert!(notice.text.contains("`kuru memory --help`"));
            assert_eq!(
                notice.indicator(),
                format!("Memory ready · kept until removed · /status · {directory}")
            );
            assert_eq!(
                status_line(&directory),
                format!("Memory: {directory} · kept until removed · kuru memory --help")
            );
            notice.record().await.unwrap();
            assert!(
                MemoryNotice::pending(store.clone())
                    .await
                    .unwrap()
                    .is_none()
            );

            let key = notice.key.clone();
            store.put(&key, &json!({ "version": 0 })).await.unwrap();
            assert!(
                MemoryNotice::pending(store.clone())
                    .await
                    .unwrap()
                    .is_some()
            );

            store
                .put(&key, &json!({ "version": VERSION + 1 }))
                .await
                .unwrap();
            assert!(
                MemoryNotice::pending(store.clone())
                    .await
                    .unwrap()
                    .is_none()
            );

            store
                .put(&key, &json!({ "version": "invalid" }))
                .await
                .unwrap();
            let error = MemoryNotice::pending(store)
                .await
                .err()
                .expect("malformed state");
            assert!(format!("{error:#}").contains("numeric version"));
            shutdown.close().await.unwrap();
            assert!(!cleanup_directory.exists());
        })
        .await
    }

    #[tokio::test]
    async fn failed_headless_write_or_flush_does_not_record_the_notice() {
        kuru_memory::test_support::closing(async {
            for fail_flush in [false, true] {
                let store = {
                    // Held across the supervisor and engine spawns; see `crate::spawn_gate`.
                    let _gate = crate::spawn_gate::spawning().await;
                    MemoryStore::temporary().await.unwrap()
                };
                let shutdown = store.clone();
                let cleanup_directory = store.status().await.unwrap().directory;
                let notice = MemoryNotice::pending(store.clone()).await.unwrap().unwrap();
                let error = notice
                    .announce_to(&mut FailingNoticeWriter { fail_flush })
                    .await
                    .unwrap_err();
                assert!(format!("{error:#}").contains("notice"));
                assert!(MemoryNotice::pending(store).await.unwrap().is_some());
                shutdown.close().await.unwrap();
                assert!(!cleanup_directory.exists());
            }
        })
        .await
    }
}
