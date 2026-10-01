//! Aged-store fixture: grow an existing isolated project store to a stated
//! logical size through the same facade write paths an ordinary turn uses,
//! so open-time measurement can compare a fresh store with an aged one.
//!
//! It is a measurement aid, compiled only with test support, and reached
//! through the `age-store` subcommand of this package's own binary (the
//! `measure:age-store` mise task). It never runs in `test`, coverage or CI.
//!
//! Every conversation follows the runtime's order for one new session and
//! `turns` single-invocation turns:
//!
//! - `create_session` (empty label), `mark_new_session`;
//! - per turn: the public admission checkpoint (user entry and turn journal),
//!   the possible-dispatch journal `put`, the actor's private input append,
//!   ledger `admit`, one terminal `observe`, `settle`, the actor's private
//!   output append, and the public settlement checkpoint (assistant entry and
//!   completed journal).
//!
//! That is `2 + 8·turns` writes and `1 + 3·turns` owned usage-ledger rows (the
//! session marker, then a record, an observation and a session-index row per
//! invocation) per conversation. The same seed, size and turn count give the
//! same logical content: identifiers, labels, transcripts, journals and usage
//! numbers. Dolt commit hashes still differ between runs, because commits
//! carry timestamps.

use crate::{MemoryStore, OpenOptions, PublicTurnSettlement, SessionTurnCheckpoint};
use anyhow::{Context, Result, bail, ensure};
use kuru_core::{
    InvocationOutcome, InvocationStart, Message, Mode, ModeProfile, Usage, UsageObservation,
    UsagePhase,
};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    time::Instant,
};

/// The JSON report's format tag.
pub const REPORT_FORMAT: &str = "kuru.aged-store";
const REPORT_FORMAT_VERSION: u32 = 1;
/// Turns beyond this are not a realistic single conversation for the open
/// measurement and would only slow the fixture.
const MAX_TURNS: u32 = 64;
const MAX_CONVERSATIONS: u64 = 1_000_000;
const PROGRESS_EVERY: u64 = 500;
/// The demo provider's route and model, which the measured release binary uses.
const ROUTE: &str = "demo";
const MODEL: &str = "demo";

/// What to add to the store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Plan {
    pub seed: u64,
    pub conversations: u64,
    pub turns: u32,
}

/// The logical volume one aging run added. Equal plans give equal counts.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Counts {
    pub seed: u64,
    pub conversations: u64,
    pub turns: u32,
    pub invocations: u64,
    pub usage_rows: u64,
    pub writes: u64,
}

impl Counts {
    /// The volume `plan` adds, from the per-conversation formulas.
    pub fn expected(plan: &Plan) -> Self {
        let turns = u64::from(plan.turns);
        Self {
            seed: plan.seed,
            conversations: plan.conversations,
            turns: plan.turns,
            invocations: plan.conversations * turns,
            usage_rows: plan.conversations * (1 + 3 * turns),
            writes: plan.conversations * (2 + 8 * turns),
        }
    }
}

/// The one stdout line `age-store` prints.
#[derive(Debug, Serialize)]
struct Report {
    format: &'static str,
    format_version: u32,
    #[serde(flatten)]
    counts: Counts,
    elapsed_ms: u64,
}

/// Encode the report line for `counts` aged in `elapsed_ms`.
pub fn report_line(counts: Counts, elapsed_ms: u64) -> Result<String> {
    Ok(serde_json::to_string(&Report {
        format: REPORT_FORMAT,
        format_version: REPORT_FORMAT_VERSION,
        counts,
        elapsed_ms,
    })?)
}

/// Parse `--data-dir <dir> --conversations <n> [--turns <n>] [--seed <n>]`.
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<(PathBuf, Plan)> {
    let mut data_dir = None;
    let mut conversations = None;
    let mut turns = None;
    let mut seed = None;
    let mut args = args.into_iter();
    while let Some(flag) = args.next() {
        let flag = flag
            .into_string()
            .map_err(|_| anyhow::anyhow!("age-store flags must be UTF-8"))?;
        let value = args
            .next()
            .with_context(|| format!("age-store {flag} needs a value"))?;
        let slot = match flag.as_str() {
            "--data-dir" => {
                ensure!(data_dir.is_none(), "age-store --data-dir given twice");
                data_dir = Some(PathBuf::from(value));
                continue;
            }
            "--conversations" => &mut conversations,
            "--turns" => &mut turns,
            "--seed" => &mut seed,
            _ => bail!("age-store does not accept {flag}"),
        };
        ensure!(slot.is_none(), "age-store {flag} given twice");
        let text = value
            .into_string()
            .map_err(|_| anyhow::anyhow!("age-store {flag} must be UTF-8"))?;
        *slot = Some(
            text.parse::<u64>()
                .with_context(|| format!("age-store {flag} must be a non-negative integer"))?,
        );
    }
    let data_dir = data_dir.context("age-store needs --data-dir")?;
    ensure!(
        data_dir.is_absolute(),
        "age-store --data-dir must be absolute"
    );
    let conversations = conversations.context("age-store needs --conversations")?;
    ensure!(
        (1..=MAX_CONVERSATIONS).contains(&conversations),
        "age-store --conversations must be between 1 and {MAX_CONVERSATIONS}"
    );
    let turns = turns.unwrap_or(1);
    ensure!(
        (1..=u64::from(MAX_TURNS)).contains(&turns),
        "age-store --turns must be between 1 and {MAX_TURNS}"
    );
    Ok((
        data_dir,
        Plan {
            seed: seed.unwrap_or(1),
            conversations,
            turns: u32::try_from(turns)?,
        },
    ))
}

/// The `age-store` subcommand: age the one store under `--data-dir` and print
/// one JSON report line. Progress goes to stderr.
pub async fn main(args: impl IntoIterator<Item = OsString>) -> Result<()> {
    let (data_dir, plan) = parse(args)?;
    let supervisor = super::prepare_supervisor()?;
    let started = Instant::now();
    let claim = claim(&data_dir).await?;
    let counts = age_claimed(claim, &plan, supervisor, None, &mut |done, writes| {
        if done % PROGRESS_EVERY == 0 || done == plan.conversations {
            let seconds = started.elapsed().as_secs_f64();
            eprintln!(
                "age-store: {done}/{} conversations, {writes} writes, {:.1} writes/s",
                plan.conversations,
                writes as f64 / seconds.max(f64::EPSILON)
            );
        }
    })
    .await?;
    let elapsed_ms = u64::try_from(started.elapsed().as_millis())?;
    println!("{}", report_line(counts, elapsed_ms)?);
    Ok(())
}

/// The store this run ages, with its project owner lock held, so no managed
/// owner can start on it until the run releases it.
pub struct Claim {
    data_dir: PathBuf,
    scope: String,
    lock: super::HeldOwnerLock,
}

impl Claim {
    pub fn scope(&self) -> &str {
        &self.scope
    }
}

/// Find the one project store under `data_dir`, wait for any previous owner
/// to release it, then hold its owner lock.
pub async fn claim(data_dir: &Path) -> Result<Claim> {
    let mut scopes = super::managed_store_scopes(data_dir)?;
    ensure!(
        scopes.len() == 1,
        "age-store needs exactly one project store under the data directory, found {}; \
         create it with one kuru run first",
        scopes.len()
    );
    let scope = scopes.pop().expect("one scope");
    let options = OpenOptions::new(data_dir.to_path_buf(), scope.clone());
    super::await_owner_release(&options).await?;
    let lock = super::hold_owner_lock(&options)?;
    Ok(Claim {
        data_dir: data_dir.to_path_buf(),
        scope,
        lock,
    })
}

/// Open the claimed store directly (offline, with `supervisor`, and the CLI's
/// own engine cache unless `cache_dir` names another), age it, close it and
/// release the owner lock. `progress` sees (conversations done, writes).
pub async fn age_claimed(
    claim: Claim,
    plan: &Plan,
    supervisor: PathBuf,
    cache_dir: Option<PathBuf>,
    progress: &mut (dyn FnMut(u64, u64) + Send),
) -> Result<Counts> {
    let mut options = OpenOptions::new(claim.data_dir.clone(), claim.scope.clone());
    options.config.offline = true;
    options.config.cache_dir = cache_dir;
    options.supervisor = Some(supervisor);
    let aged = async {
        let memory = MemoryStore::open(options).await?;
        let aged = age_open_store(&memory, &claim.scope, plan, progress).await;
        let closed = memory.close().await;
        let counts = aged?;
        closed?;
        Ok::<_, anyhow::Error>(counts)
    }
    .await;
    let released = claim.lock.release();
    let counts = aged?;
    released?;
    Ok(counts)
}

/// Add `plan`'s conversations to an open writable store of `scope`.
pub async fn age_open_store(
    memory: &MemoryStore,
    scope: &str,
    plan: &Plan,
    progress: &mut (dyn FnMut(u64, u64) + Send),
) -> Result<Counts> {
    let profile = ModeProfile::builtin(Mode::Ifs);
    let identity = profile
        .roles
        .seeds()
        .into_iter()
        .next()
        .context("the built-in IFS profile has no part")?
        .id;
    let ledger = memory.usage_ledger()?;
    let mut counts = Counts {
        seed: plan.seed,
        conversations: 0,
        turns: plan.turns,
        invocations: 0,
        usage_rows: 0,
        writes: 0,
    };
    for index in 0..plan.conversations {
        let mut random = SplitMix64::new(plan.seed, index);
        let session_id = random.uuid();
        let operation_id = random.uuid();
        let transcript = profile.memory.transcript_namespace(scope, &session_id);
        ensure!(
            transcript == format!("{scope}/transcript/{session_id}"),
            "the built-in profile changed its transcript namespace"
        );
        let actor = profile
            .memory
            .identity_namespace(scope, Mode::Ifs, &identity);
        let created = memory.create_session(&session_id, Mode::Ifs, "").await?;
        ledger.mark_new_session(&session_id).await?;
        counts.writes += 2;
        counts.usage_rows += 1;
        let generation = created.lifecycle_generation;
        for turn in 0..plan.turns {
            let turn_id = random.uuid();
            let prompt = random.text(200, 2_000);
            let answer = random.text(500, 6_000);
            let journal_key = format!(
                "{scope}/session/{session_id}/turn/{}",
                hex(&Sha256::new()
                    .chain_update(b"kuru.turn-journal.v1\0")
                    .chain_update(turn_id.as_bytes())
                    .finalize())
            );
            let rows = memory.history_window(&transcript, 0).await?.total_rows;
            let mut journal = json!({
                "format": 2,
                "id": turn_id,
                "prompt": prompt,
                "target": null,
                "transitions": ["started"],
                "possible_dispatch": false,
                "interruption_marker": false,
                "admitted_transcript_row": rows + 1,
                "output": null,
            });
            memory
                .checkpoint_session_turn(
                    &transcript,
                    &session_id,
                    &[Message::text("user", &prompt)],
                    &[(journal_key.clone(), journal.clone())],
                    &SessionTurnCheckpoint::Admit {
                        expected_generation: generation,
                        turn_id: turn_id.clone(),
                        label: (turn == 0).then(|| prompt.chars().take(80).collect()),
                        expected_transcript_rows: Some(rows),
                    },
                )
                .await?;
            journal["possible_dispatch"] = json!(true);
            journal["transitions"] = json!(["started", "possible_dispatch"]);
            memory.put(&journal_key, &journal).await?;
            memory
                .append_session_message(&actor, &session_id, &Message::text("user", &prompt))
                .await?;
            let invocation_id = format!("v1-{}", hex(&random.bytes::<32>()));
            ledger
                .admit(InvocationStart {
                    session_id: session_id.clone(),
                    invocation_id: invocation_id.clone(),
                    operation_id: operation_id.clone(),
                    phase: UsagePhase::Speak,
                    actor_id: identity.clone(),
                    route: ROUTE.into(),
                    model: MODEL.into(),
                    price_at_invocation: None,
                })
                .await?;
            let input = random.below(4_000) + 200;
            ledger
                .observe(
                    &invocation_id,
                    UsageObservation {
                        sequence: 1,
                        terminal: true,
                        usage: Usage {
                            input_tokens: Some(input),
                            output_tokens: Some(random.below(1_500) + 50),
                            cached_input_tokens: Some(random.below(input)),
                            reasoning_output_tokens: Some(random.below(500)),
                        },
                    },
                )
                .await?;
            ledger
                .settle(&invocation_id, InvocationOutcome::Succeeded)
                .await?;
            memory
                .append_session_message(&actor, &session_id, &Message::text("assistant", &answer))
                .await?;
            journal["transitions"] = json!(["started", "possible_dispatch", "ended"]);
            journal["output"] = json!({ "text": answer });
            memory
                .checkpoint_session_turn(
                    &transcript,
                    &session_id,
                    &[Message::text("assistant", &answer)],
                    &[(journal_key, journal)],
                    &SessionTurnCheckpoint::Settle {
                        expected_generation: generation,
                        turn_id,
                        settlement: PublicTurnSettlement::Completed,
                        speaker_id: identity.clone(),
                    },
                )
                .await?;
            counts.writes += 8;
            counts.usage_rows += 3;
            counts.invocations += 1;
        }
        counts.conversations += 1;
        progress(counts.conversations, counts.writes);
    }
    Ok(counts)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// SplitMix64, seeded per conversation from `(seed, index)`, so a
/// conversation's content does not depend on the ones before it.
struct SplitMix64(u64);

impl SplitMix64 {
    fn new(seed: u64, index: u64) -> Self {
        let mut mixer = Self(seed);
        let base = mixer.next();
        Self(base ^ index.wrapping_mul(0xd1b5_4a32_d192_ed03))
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `0..bound` (bound > 0), by rejection.
    fn below(&mut self, bound: u64) -> u64 {
        let zone = u64::MAX - u64::MAX % bound;
        loop {
            let value = self.next();
            if value < zone {
                return value % bound;
            }
        }
    }

    fn bytes<const N: usize>(&mut self) -> [u8; N] {
        let mut bytes = [0; N];
        for chunk in bytes.chunks_mut(8) {
            chunk.copy_from_slice(&self.next().to_le_bytes()[..chunk.len()]);
        }
        bytes
    }

    fn uuid(&mut self) -> String {
        uuid::Builder::from_random_bytes(self.bytes())
            .into_uuid()
            .to_string()
    }

    /// ASCII pseudo-text of a uniform length in `min..=max` bytes.
    fn text(&mut self, min: u64, max: u64) -> String {
        const WORDS: [&str; 16] = [
            "the", "part", "memory", "quiet", "river", "holds", "a", "small", "light", "and",
            "returns", "when", "asked", "slowly", "over", "time",
        ];
        let length = usize::try_from(min + self.below(max - min + 1)).expect("bounded length");
        let mut text = String::with_capacity(length + 8);
        while text.len() < length {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(WORDS[usize::try_from(self.below(16)).expect("word index")]);
        }
        text.truncate(length);
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{FixtureDeadline, fixture_deadline, tempdir};

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    /// A path that is absolute on the host: `/` is not absolute on Windows,
    /// where a path needs a drive prefix, and the parser must keep refusing
    /// relative paths on both.
    fn absolute(name: &str) -> String {
        #[cfg(windows)]
        const ROOT: &str = "C:\\";
        #[cfg(not(windows))]
        const ROOT: &str = "/";
        format!("{ROOT}{name}")
    }

    #[test]
    fn parse_reads_flags_and_defaults() -> Result<()> {
        let d = absolute("d");
        let d = d.as_str();
        let (data, plan) = parse(args(&["--data-dir", d, "--conversations", "5"]))?;
        assert_eq!(data, PathBuf::from(d));
        assert_eq!(
            plan,
            Plan {
                seed: 1,
                conversations: 5,
                turns: 1
            }
        );
        let (_, plan) = parse(args(&[
            "--seed",
            "9",
            "--turns",
            "3",
            "--conversations",
            "2",
            "--data-dir",
            d,
        ]))?;
        assert_eq!(
            plan,
            Plan {
                seed: 9,
                conversations: 2,
                turns: 3
            }
        );
        Ok(())
    }

    #[test]
    fn parse_refuses_malformed_arguments() {
        let (d, e) = (absolute("d"), absolute("e"));
        let (d, e) = (d.as_str(), e.as_str());
        for (input, expected) in [
            (vec!["--conversations", "1"], "needs --data-dir"),
            (vec!["--data-dir", d], "needs --conversations"),
            (
                vec!["--data-dir", "relative", "--conversations", "1"],
                "absolute",
            ),
            (vec!["--data-dir", d, "--conversations", "0"], "between"),
            (vec!["--data-dir", d, "--conversations", "x"], "integer"),
            (vec!["--data-dir", d, "--conversations", "-1"], "integer"),
            (
                vec!["--data-dir", d, "--conversations", "1", "--turns", "0"],
                "between",
            ),
            (
                vec!["--data-dir", d, "--conversations", "1", "--turns", "65"],
                "between",
            ),
            (
                vec!["--data-dir", d, "--conversations", "1", "--seed"],
                "needs a value",
            ),
            (
                vec!["--data-dir", d, "--conversations", "1", "--bogus", "1"],
                "does not accept",
            ),
            (
                vec!["--data-dir", d, "--data-dir", e, "--conversations", "1"],
                "twice",
            ),
            (
                vec![
                    "--data-dir",
                    d,
                    "--conversations",
                    "1",
                    "--conversations",
                    "1",
                ],
                "twice",
            ),
        ] {
            let error = parse(args(&input)).unwrap_err();
            assert!(
                format!("{error:#}").contains(expected),
                "{input:?}: {error:#}"
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            let invalid = OsString::from_vec(vec![0xff]);
            let error = parse(vec![invalid.clone(), "x".into()]).unwrap_err();
            assert!(format!("{error:#}").contains("UTF-8"), "{error:#}");
            let error = parse(vec![
                "--data-dir".into(),
                d.into(),
                "--conversations".into(),
                invalid,
            ])
            .unwrap_err();
            assert!(format!("{error:#}").contains("UTF-8"), "{error:#}");
        }
    }

    #[test]
    fn the_report_line_has_its_format_and_exact_fields() -> Result<()> {
        let plan = Plan {
            seed: 7,
            conversations: 3,
            turns: 2,
        };
        let line = report_line(Counts::expected(&plan), 1234)?;
        let value: serde_json::Value = serde_json::from_str(&line)?;
        assert_eq!(
            value,
            json!({
                "format": "kuru.aged-store",
                "format_version": 1,
                "seed": 7,
                "conversations": 3,
                "turns": 2,
                "invocations": 6,
                "usage_rows": 21,
                "writes": 54,
                "elapsed_ms": 1234,
            })
        );
        assert!(!line.contains('\n'));
        Ok(())
    }

    #[test]
    fn generated_content_is_a_pure_function_of_seed_and_index() {
        let sample = |seed, index| {
            let mut random = SplitMix64::new(seed, index);
            (random.uuid(), random.text(200, 2_000), random.below(10))
        };
        assert_eq!(sample(7, 3), sample(7, 3));
        assert_ne!(sample(7, 3), sample(7, 4));
        assert_ne!(sample(7, 3), sample(8, 3));
        let mut random = SplitMix64::new(1, 0);
        for _ in 0..200 {
            let text = random.text(200, 2_000);
            assert!((200..=2_000).contains(&text.len()) && text.is_ascii());
            let id = uuid::Uuid::parse_str(&random.uuid()).expect("valid uuid");
            assert_eq!(id.get_version_num(), 4);
        }
    }

    /// Everything a reader can observe of one aged store's sessions.
    async fn observed(memory: &MemoryStore, scope: &str) -> Result<Vec<serde_json::Value>> {
        let page = memory.session_catalog_page(None, None, None, 64).await?;
        ensure!(page.next.is_none(), "fixture catalog spans pages");
        let ledger = memory.usage_ledger()?;
        let mut sessions = Vec::new();
        for record in page.records {
            let id = &record.session_id;
            let usage = ledger.session(id).await?;
            let transcript = memory
                .history_window(&format!("{scope}/transcript/{id}"), 64)
                .await?;
            sessions.push(json!({
                "id": id,
                "label": record.label,
                "invocations": usage.invocation_count,
                "complete": usage.historical_complete,
                "usage": usage.known_usage,
                "transcript": transcript.messages,
                "rows": transcript.total_rows,
            }));
        }
        sessions.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
        Ok(sessions)
    }

    /// Test 10: equal plans age two template stores to equal logical content
    /// and the stated counts; another seed gives other sessions.
    #[tokio::test]
    async fn equal_plans_age_equal_content_with_the_stated_counts() -> Result<()> {
        let plan = Plan {
            seed: 7,
            conversations: 3,
            turns: 2,
        };
        let first = MemoryStore::temporary().await?;
        let second = MemoryStore::temporary().await?;
        let other = MemoryStore::temporary().await?;
        let scope = crate::store::temporary_scope();
        let deadline = FixtureDeadline::start(fixture_deadline(0, 0), "aged store determinism");
        let outcome = deadline
            .run(async {
                let mut seen = Vec::new();
                let counts = age_open_store(&first, &scope, &plan, &mut |done, writes| {
                    seen.push((done, writes));
                })
                .await?;
                assert_eq!(counts, Counts::expected(&plan));
                assert_eq!((counts.writes, counts.usage_rows), (54, 21));
                assert_eq!(seen, vec![(1, 18), (2, 36), (3, 54)]);
                let again = age_open_store(&second, &scope, &plan, &mut |_, _| {}).await?;
                assert_eq!(again, counts);
                let observed_first = observed(&first, &scope).await?;
                assert_eq!(observed_first.len(), 3);
                for session in &observed_first {
                    assert_eq!(session["invocations"], json!(2));
                    assert_eq!(session["complete"], json!(true));
                    assert_eq!(session["rows"], json!(4));
                    assert!(!session["label"].as_str().unwrap_or_default().is_empty());
                }
                assert_eq!(observed_first, observed(&second, &scope).await?);
                let reseeded = Plan { seed: 8, ..plan };
                age_open_store(&other, &scope, &reseeded, &mut |_, _| {}).await?;
                let ids = |sessions: &[serde_json::Value]| {
                    sessions
                        .iter()
                        .map(|session| session["id"].clone())
                        .collect::<Vec<_>>()
                };
                let observed_other = observed(&other, &scope).await?;
                assert!(
                    ids(&observed_other)
                        .iter()
                        .all(|id| !ids(&observed_first).contains(id))
                );
                Ok::<_, anyhow::Error>(())
            })
            .await;
        let closed = (
            first.close().await,
            second.close().await,
            other.close().await,
        );
        outcome?;
        closed.0?;
        closed.1?;
        closed.2
    }

    /// The wrapper: a store at a real data directory is found, claimed under
    /// its owner lock, aged through the direct open and released; its counts
    /// equal the inner function's for the same plan.
    #[tokio::test]
    async fn the_claimed_wrapper_ages_the_one_store_and_releases_its_lock() -> Result<()> {
        let root = tempdir()?;
        let data_dir = root.path().join("private");
        let options =
            super::super::warmed_open_options(data_dir.clone(), crate::store::temporary_scope())
                .await?;
        let deadline = FixtureDeadline::start(fixture_deadline(1, 1), "aged store wrapper");
        let plan = Plan {
            seed: 7,
            conversations: 2,
            turns: 1,
        };
        let outcome = deadline
            .run(async {
                let gate = crate::spawn_gate::spawning().await;
                MemoryStore::open(options.clone()).await?.close().await?;
                // Claiming waits for and then takes the owner lock right after
                // the seed's release: exclude sibling spawns across it.
                let (claim, gate) = crate::spawn_gate::excluding_spawns(gate, async {
                    let claim = claim(&data_dir).await?;
                    ensure!(claim.scope() == options.project_scope);
                    Ok(claim)
                })
                .await?;
                let mut done = 0;
                let counts = age_claimed(
                    claim,
                    &plan,
                    crate::store::test_supervisor()?,
                    Some(crate::store::test_cache()),
                    &mut |conversations, _| done = conversations,
                )
                .await?;
                assert_eq!(counts, Counts::expected(&plan));
                assert_eq!(done, 2);
                let (_, gate) = crate::spawn_gate::excluding_spawns(gate, async {
                    super::super::hold_owner_lock(&options)?.release()
                })
                .await?;
                drop(gate);
                Ok::<_, anyhow::Error>(())
            })
            .await;
        root.release(outcome)
    }
}
