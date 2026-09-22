//! App-owned, actor-only bridge between checked native targets and workspace
//! instruction trust. Permission approval and prompt authority stay separate.

use std::{path::PathBuf, sync::Arc};

use anyhow::{Result, bail, ensure};
use async_trait::async_trait;
use kuru_connectors::{
    InstructionActivation, InstructionGate, InstructionGateOutcome, InstructionReviewAnswer,
    InstructionReviewSender,
};
use kuru_core::{ConfigSnapshot, ProjectRelativeTarget};
use kuru_platform::fs::Directory;
use tokio::sync::Mutex;

use crate::{
    cli::{all_claim_categories, manifest_text},
    trust::{ApprovalMethod, ApprovalState, ApprovalStore, ReviewGeneration},
};

struct ActiveSnapshot {
    value: ConfigSnapshot,
    generation: u64,
}

pub(crate) struct NestedInstructionGate {
    root: Arc<Directory>,
    data: PathBuf,
    base: ConfigSnapshot,
    active: Arc<Mutex<ActiveSnapshot>>,
    invocation_once: bool,
}

impl NestedInstructionGate {
    pub(crate) fn new(
        root: Arc<Directory>,
        data: PathBuf,
        base: ConfigSnapshot,
        invocation_once: bool,
    ) -> Self {
        Self {
            root,
            data,
            active: Arc::new(Mutex::new(ActiveSnapshot {
                value: base.clone(),
                generation: 0,
            })),
            base,
            invocation_once,
        }
    }
}

struct ProposedActivation {
    root: Arc<Directory>,
    data: PathBuf,
    base: ConfigSnapshot,
    active: Arc<Mutex<ActiveSnapshot>>,
    prior_generation: u64,
    snapshot: ConfigSnapshot,
    source_paths: Vec<[u8; 32]>,
    reviewed: Option<ReviewGeneration>,
    persist: bool,
}

#[async_trait]
impl InstructionGate for NestedInstructionGate {
    async fn review(
        &self,
        directories: &[ProjectRelativeTarget],
        approval: Option<&InstructionReviewSender>,
    ) -> Result<InstructionGateOutcome> {
        let (current, prior_generation) = {
            let active = self.active.lock().await;
            (active.value.clone(), active.generation)
        };
        let directories = directories
            .iter()
            .map(|directory| PathBuf::from(directory.as_str()))
            .collect::<Vec<_>>();
        let (snapshot, changed) = current.with_nested_directories(&directories)?;
        if !changed {
            return Ok(InstructionGateOutcome::Unchanged);
        }
        let source_paths = snapshot.nested_instruction_source_paths();
        ensure!(
            !source_paths.is_empty(),
            "nested instruction manifest lacks a source set"
        );
        let store = ApprovalStore::new(&self.data, &self.root);
        let (state, reviewed) =
            store.inspect_nested(self.base.manifest(), snapshot.manifest(), &source_paths);
        let answer = if state == ApprovalState::Matching {
            InstructionReviewAnswer::Once
        } else if let Some(approval) = approval {
            let complete = snapshot.manifest().filtered(&all_claim_categories());
            let display = format!(
                "New path-specific project instructions require workspace trust.\n{}",
                manifest_text(&self.root, &complete, Some(state))
            );
            let Some(answer) = approval.ask(display, reviewed.is_some()).await else {
                return Ok(InstructionGateOutcome::Required(
                    "foreground instruction review is unavailable; retry with an active terminal or use --trust-workspace-once after reviewing workspace authority".into(),
                ));
            };
            answer
        } else if self.invocation_once {
            InstructionReviewAnswer::Once
        } else {
            return Ok(InstructionGateOutcome::Required(
                "nested project instructions need workspace trust; retry in the terminal or use --trust-workspace-once after reviewing the complete manifest".into(),
            ));
        };
        match answer {
            InstructionReviewAnswer::Deny => Ok(InstructionGateOutcome::Denied),
            InstructionReviewAnswer::Once | InstructionReviewAnswer::Persist => {
                if answer == InstructionReviewAnswer::Persist && reviewed.is_none() {
                    bail!(
                        "workspace approval state is unsafe; use a one-time review or revoke it before persisting new authority"
                    );
                }
                Ok(InstructionGateOutcome::Proposed(Box::new(
                    ProposedActivation {
                        root: self.root.clone(),
                        data: self.data.clone(),
                        base: self.base.clone(),
                        active: self.active.clone(),
                        prior_generation,
                        snapshot,
                        source_paths,
                        reviewed,
                        persist: answer == InstructionReviewAnswer::Persist,
                    },
                )))
            }
        }
    }
}

#[async_trait]
impl InstructionActivation for ProposedActivation {
    async fn publish(self: Box<Self>) -> Result<String> {
        let mut active = self.active.lock().await;
        ensure!(
            active.generation == self.prior_generation,
            "project instructions changed while review was pending; retry the tool"
        );
        self.root.revalidate()?;
        self.snapshot.revalidate_nested_directories()?;
        if self.persist {
            ApprovalStore::new(&self.data, &self.root).approve_nested(
                self.base.manifest(),
                self.snapshot.manifest(),
                &self.source_paths,
                self.reviewed
                    .as_ref()
                    .expect("persistent review carries an approval generation"),
                ApprovalMethod::InteractiveTui,
            )?;
        }
        let instructions = self.snapshot.instructions().to_owned();
        active.value = self.snapshot;
        active.generation = active.generation.wrapping_add(1);
        Ok(instructions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuru_connectors::{InstructionReviewRequest, ToolHost};
    use kuru_core::{Config, InvocationOverrides};
    use serde_json::json;

    #[tokio::test]
    async fn headless_nested_review_precedes_file_effect_and_direct_tools_stay_independent() {
        let project = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        std::fs::create_dir(project.path().join("src")).unwrap();
        std::fs::create_dir(project.path().join("sibling")).unwrap();
        std::fs::write(project.path().join("src/AGENTS.md"), "src instruction").unwrap();
        std::fs::write(
            project.path().join("sibling/AGENTS.md"),
            "sibling instruction",
        )
        .unwrap();
        let root = Arc::new(
            Directory::open(
                project.path(),
                kuru_platform::fs::Privacy::Inherited,
                kuru_platform::fs::NameRetention::Pinned,
            )
            .unwrap(),
        );
        let snapshot = ConfigSnapshot::parse(
            None,
            project.path(),
            None,
            InvocationOverrides {
                allow_write: true,
                ..Default::default()
            },
        )
        .unwrap();
        let config = Config {
            allow_write: true,
            ..Default::default()
        };
        let gated = ToolHost::with_retained_root(root.clone(), &config)
            .unwrap()
            .with_instruction_gate(Arc::new(NestedInstructionGate::new(
                root.clone(),
                data.path().to_path_buf(),
                snapshot.clone(),
                false,
            )));
        let refused = gated
            .execute_for_actor(
                "file_write",
                json!({"path":"src/blocked.txt","content":"no effect"}),
                None,
                None,
            )
            .await;
        assert!(
            refused
                .result
                .unwrap_err()
                .to_string()
                .contains("nested project instructions")
        );
        assert!(!project.path().join("src/blocked.txt").exists());
        gated
            .execute(
                "file_write",
                json!({"path":"src/direct.txt","content":"direct"}),
            )
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(project.path().join("src/direct.txt")).unwrap(),
            "direct"
        );

        let once = ToolHost::with_retained_root(root.clone(), &config)
            .unwrap()
            .with_instruction_gate(Arc::new(NestedInstructionGate::new(
                root,
                data.path().to_path_buf(),
                snapshot,
                true,
            )));
        let first = once
            .execute_for_actor(
                "file_write",
                json!({"path":"src/approved.txt","content":"approved"}),
                None,
                None,
            )
            .await;
        assert!(first.replan_required);
        assert!(!project.path().join("src/approved.txt").exists());
        let instructions = first.instructions.unwrap();
        assert!(instructions.contains("src instruction"));
        assert!(!instructions.contains("sibling instruction"));
        let second = once
            .execute_for_actor(
                "file_write",
                json!({"path":"src/approved.txt","content":"approved"}),
                None,
                None,
            )
            .await;
        assert!(!second.replan_required);
        assert!(second.instructions.is_none());
        second.result.unwrap();
        assert_eq!(
            std::fs::read_to_string(project.path().join("src/approved.txt")).unwrap(),
            "approved"
        );
        std::fs::write(project.path().join("sibling/note.txt"), "sibling data").unwrap();
        let sibling = once
            .execute_for_actor("file_read", json!({"path":"sibling/note.txt"}), None, None)
            .await;
        assert!(!sibling.replan_required);
        let instructions = sibling.instructions.unwrap();
        assert!(instructions.contains("src instruction"));
        assert!(instructions.contains("sibling instruction"));
        assert!(sibling.result.unwrap().contains("sibling data"));
    }

    #[tokio::test]
    async fn foreground_persistent_review_publishes_only_after_the_tool_gate() {
        let project = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let state = data.path().join("state");
        std::fs::create_dir(project.path().join("src")).unwrap();
        std::fs::write(
            project.path().join("src/AGENTS.md"),
            "reviewed path instruction",
        )
        .unwrap();
        let root = Arc::new(
            Directory::open(
                project.path(),
                kuru_platform::fs::Privacy::Inherited,
                kuru_platform::fs::NameRetention::Pinned,
            )
            .unwrap(),
        );
        let base =
            ConfigSnapshot::parse(None, project.path(), None, InvocationOverrides::default())
                .unwrap();
        let gate = Arc::new(NestedInstructionGate::new(
            root.clone(),
            state.clone(),
            base.clone(),
            false,
        ));
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<InstructionReviewRequest>(1);
        let pending = tokio::spawn(async move {
            gate.review(
                &[ProjectRelativeTarget::parse("src").unwrap()],
                Some(&InstructionReviewSender::new(sender)),
            )
            .await
        });
        let request = tokio::time::timeout(std::time::Duration::from_secs(5), receiver.recv())
            .await
            .expect("nested review request deadline")
            .unwrap();
        assert!(request.persistent_allowed, "{}", request.display);
        assert!(!request.display.contains("reviewed path instruction"));
        assert!(request.display.contains("Manifest:"));
        assert_eq!(
            ApprovalStore::new(&state, &root).inspect(base.manifest()),
            ApprovalState::Absent
        );
        std::fs::write(
            project.path().join("src/AGENTS.md"),
            "changed after capture",
        )
        .unwrap();
        request
            .reply
            .send(InstructionReviewAnswer::Persist)
            .unwrap();
        let InstructionGateOutcome::Proposed(proposed) =
            tokio::time::timeout(std::time::Duration::from_secs(5), pending)
                .await
                .expect("nested review reply deadline")
                .unwrap()
                .unwrap()
        else {
            panic!("persistent answer did not produce a deferred activation");
        };
        assert_eq!(
            ApprovalStore::new(&state, &root).inspect(base.manifest()),
            ApprovalState::Absent,
            "foreground answer alone must not publish approval before target revalidation"
        );
        let prompt = proposed.publish().await.unwrap();
        assert!(prompt.contains("reviewed path instruction"));
        assert!(!prompt.contains("changed after capture"));
        assert_eq!(
            ApprovalStore::new(&state, &root).inspect(base.manifest()),
            ApprovalState::Matching
        );
        let fresh =
            ConfigSnapshot::parse(None, project.path(), None, InvocationOverrides::default())
                .unwrap()
                .with_nested_directories(&["src".into()])
                .unwrap()
                .0;
        assert!(fresh.instructions().contains("changed after capture"));
        assert_eq!(
            ApprovalStore::new(&state, &root)
                .inspect_nested(
                    base.manifest(),
                    fresh.manifest(),
                    &fresh.nested_instruction_source_paths(),
                )
                .0,
            ApprovalState::Absent,
            "changed bytes no longer match the stored nested entry and require a new review"
        );
    }

    #[tokio::test]
    async fn foreground_review_rejects_replaced_nested_directory_before_publication() {
        let project = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let state = data.path().join("state");
        std::fs::create_dir(project.path().join("src")).unwrap();
        std::fs::write(project.path().join("src/AGENTS.md"), "original instruction").unwrap();
        let root = Arc::new(
            Directory::open(
                project.path(),
                kuru_platform::fs::Privacy::Inherited,
                kuru_platform::fs::NameRetention::Pinned,
            )
            .unwrap(),
        );
        let base =
            ConfigSnapshot::parse(None, project.path(), None, InvocationOverrides::default())
                .unwrap();
        let gate = Arc::new(NestedInstructionGate::new(
            root.clone(),
            state.clone(),
            base.clone(),
            false,
        ));
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<InstructionReviewRequest>(1);
        let pending = tokio::spawn(async move {
            gate.review(
                &[ProjectRelativeTarget::parse("src").unwrap()],
                Some(&InstructionReviewSender::new(sender)),
            )
            .await
        });
        let request = tokio::time::timeout(std::time::Duration::from_secs(5), receiver.recv())
            .await
            .expect("replacement review request deadline")
            .unwrap();
        std::fs::rename(project.path().join("src"), project.path().join("retired")).unwrap();
        std::fs::create_dir(project.path().join("src")).unwrap();
        std::fs::write(
            project.path().join("src/AGENTS.md"),
            "replacement instruction",
        )
        .unwrap();
        request
            .reply
            .send(InstructionReviewAnswer::Persist)
            .unwrap();
        let InstructionGateOutcome::Proposed(proposed) =
            tokio::time::timeout(std::time::Duration::from_secs(5), pending)
                .await
                .expect("replacement review reply deadline")
                .unwrap()
                .unwrap()
        else {
            panic!("persistent answer did not produce a deferred activation");
        };
        assert!(
            proposed
                .publish()
                .await
                .unwrap_err()
                .to_string()
                .contains("nested instruction directory changed after capture")
        );
        assert_eq!(
            ApprovalStore::new(&state, &root).inspect(base.manifest()),
            ApprovalState::Absent,
            "changed instruction directory must not publish persistent approval"
        );
    }
}
