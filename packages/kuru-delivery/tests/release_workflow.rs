#![cfg(feature = "tooling")]
use axum::{
    Router,
    body::to_bytes,
    extract::{Request, State},
    http::{Method, StatusCode},
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use kuru_delivery::release::{self, GitHub, Version};
use kuru_delivery::{
    archive::digest,
    command,
    coverage::{Mode, partition_count},
    shell_support,
};
use kuru_platform::fs::make_executable;
use serde_json::{Value, json};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tempfile::TempDir;

#[path = "support/repository_environment.rs"]
mod repository_environment;

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const C: &str = "cccccccccccccccccccccccccccccccccccccccc";
fn v(input: &str) -> Version {
    input.parse().unwrap()
}

#[test]
fn required_release_checks_precede_the_only_publication_job() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    // Publish-mode behavior is asserted on the mode=publish projection, so
    // every assertion below still describes the unconditional publication.
    let workflow = publish_projection(
        &fs::read_to_string(root.join(".github/workflows/release.yml")).unwrap(),
    );
    let job = |name: &str| {
        workflow
            .split_once(&format!("\n  {name}:\n"))
            .unwrap_or_else(|| panic!("missing release job {name}"))
            .1
            .lines()
            .take_while(|line| !line.starts_with("  ") || line.starts_with("    "))
            .collect::<Vec<_>>()
            .join("\n")
    };
    // Without condition overrides, failed, cancelled, or skipped prerequisites
    // prevent their dependent jobs from running under GitHub's default policy.
    for (name, needs) in [
        ("build", "needs: [plan, bump, verify]"),
        ("assemble-candidate", "needs: [plan, bump, build, notes]"),
        ("verify-staged", "needs: [plan, bump, assemble-candidate]"),
        ("build-docs", "needs: [bump, assemble-candidate]"),
        ("deploy-docs", "needs: [build-docs, verify-staged]"),
        (
            "publish",
            "needs: [plan, bump, assemble-candidate, verify-staged, deploy-docs]",
        ),
    ] {
        let body = job(name);
        assert!(body.contains(needs), "{name} lost required dependencies");
        assert!(
            !body.lines().any(|line| line.starts_with("    if:")),
            "{name} must retain the default prerequisite success condition"
        );
        assert!(
            !body.contains("continue-on-error:"),
            "{name} must fail on unsuccessful required work"
        );
        // Native matrix jobs select their platform's steps with runner.os.
        if name != "build" && name != "verify-staged" {
            assert!(
                !body
                    .lines()
                    .any(|line| line.trim_start().starts_with("if:")),
                "{name} must not skip required steps"
            );
        }
    }
    // Planning follows static checks and one ordinary native test pass on the
    // dispatch SHA; coverage is CI's gate, never rerun inside the release.
    assert!(job("plan").contains("needs: [source-quality, tests]"));
    assert!(!workflow.contains("native-tests.yml"));
    assert!(!workflow.contains("source-tests"));
    assert!(!workflow.contains("verify-tests"));
    assert!(!workflow.contains("mise run coverage"));
    let tests = job("tests");
    for required in [
        "if: github.ref == 'refs/heads/main'",
        "runs-on: ubuntu-latest",
        "timeout-minutes: 60",
        "CARGO_PROFILE_TEST_DEBUG: \"0\"",
        "ref: ${{ github.sha }}",
        "install_args: rust github:aligned-team/cospec",
        "mise run //packages/kuru-delivery:setup:test-tools",
        "dbus-run-session -- bash -euo pipefail <<'KURU_NATIVE_STORE'",
        "            mise run test\n          KURU_NATIVE_STORE",
    ] {
        assert!(tests.contains(required), "release tests lost {required}");
    }
    assert!(!tests.contains("continue-on-error:"));
    // The OS secret-store session is the same text CI runs around each coverage shard.
    let native = fs::read_to_string(root.join(".github/workflows/native-tests.yml")).unwrap();
    let session = |text: &str, command: &str| {
        let start = text
            .find("          # The Linux keyring backend requires")
            .unwrap();
        let end = text[start..]
            .find(&format!(
                "            {command}\n          KURU_NATIVE_STORE\n"
            ))
            .unwrap();
        text[start..start + end].to_owned()
    };
    assert_eq!(
        session(&tests, "mise run test"),
        session(&native, "mise run //packages/kuru-delivery:coverage:shard")
    );

    let assembly = job("assemble-candidate");
    assert!(assembly.contains("mise run release:tool -- assemble"));
    assert!(assembly.contains("name=release-candidate-%s"));
    assert!(assembly.contains("RUN_ATTEMPT: ${{ github.run_attempt }}"));
    assert!(assembly.contains("artifact_name: ${{ steps.artifact.outputs.name }}"));
    assert!(assembly.contains("dist/*\n            RELEASE_NOTES.md"));
    assert!(assembly.contains("if-no-files-found: error"));
    assert!(!assembly.contains("contents: write"));
    assert!(!assembly.contains("GH_TOKEN:"));

    let build = job("build");
    for required in [
        "mise run //apps/kuru-tui:shell-support:generate",
        "mise run //packages/kuru-delivery:package:shell-support",
        "Get-FileHash -Algorithm SHA256",
        "openssl dgst -sha256",
        // Linux release archives keep the documented Ubuntu 24.04 glibc floor.
        "- os: ubuntu-24.04\n            target: x86_64-unknown-linux-gnu\n",
        "- os: ubuntu-24.04-arm\n            target: aarch64-unknown-linux-gnu\n",
    ] {
        assert!(build.contains(required), "native build lost {required}");
    }

    let verifier = job("verify-staged");
    // Staged acceptance runs on every supported platform before promotion.
    let matrix = verifier
        .split("        include:\n")
        .nth(1)
        .unwrap()
        .split("    runs-on:")
        .next()
        .unwrap();
    assert_eq!(
        matrix,
        "          - os: windows-latest\n            target: x86_64-pc-windows-msvc\n          - os: ubuntu-latest\n            target: x86_64-unknown-linux-gnu\n          - os: ubuntu-24.04-arm\n            target: aarch64-unknown-linux-gnu\n          - os: macos-latest\n            target: aarch64-apple-darwin\n"
    );
    for required in [
        "fail-fast: false",
        "runs-on: ${{ matrix.os }}",
        "timeout-minutes: 45",
        "ref: ${{ needs.bump.outputs.sha }}",
        "version: 2026.9.4",
        "name: ${{ needs.assemble-candidate.outputs.artifact_name }}",
        "path: candidate",
        "KURU_STAGED_WINDOWS_ARCHIVE: ${{ github.workspace }}/candidate/dist/kuru-${{ needs.plan.outputs.version }}-x86_64-pc-windows-msvc.zip",
        "mise run //apps/kuru-tui:verify:staged-windows",
        "RELEASE_ARCHIVE: kuru-${{ needs.plan.outputs.version }}-${{ matrix.target }}.tar.gz",
        "candidate/dist/SHA256SUMS",
        "shasum -a 256 -c",
        "KURU_EMBEDDED_TEST_BINARY: ${{ runner.temp }}/kuru-staged/kuru",
        "KURU_UPDATE_CANDIDATE_BINARY: ${{ runner.temp }}/kuru-staged/kuru",
        "mise run //apps/kuru-tui:test:embedded-runtime",
        "mise run //packages/kuru-delivery:test:previous-release-update",
    ] {
        assert!(verifier.contains(required), "missing {required}");
    }
    // Each platform's steps are selected by runner.os, and the Unix legs check
    // the archive checksum before extracting and running the staged executable.
    let steps = format!("\n{}", verifier.split("    steps:\n").nth(1).unwrap());
    let steps: Vec<_> = steps.split("\n      - ").skip(1).collect();
    let step = |name: &str| {
        steps
            .iter()
            .position(|step| step.starts_with(&format!("name: {name}\n")))
            .unwrap_or_else(|| panic!("missing staged step {name}"))
    };
    for (name, condition) in [
        (
            "Extract the exact staged Unix archive",
            "if: runner.os != 'Windows'",
        ),
        (
            "Verify the staged Unix executable's offline runtime",
            "if: runner.os != 'Windows'",
        ),
        (
            "Accept the staged Unix executable from the previous published release",
            "if: runner.os != 'Windows'",
        ),
        (
            "Verify the exact staged package",
            "if: runner.os == 'Windows'",
        ),
    ] {
        assert!(
            steps[step(name)].contains(condition),
            "{name} lost {condition}"
        );
    }
    let extract = steps[step("Extract the exact staged Unix archive")];
    assert!(extract.find("shasum -a 256 -c").unwrap() < extract.find("tar -xzf").unwrap());
    assert!(
        step("Extract the exact staged Unix archive")
            < step("Verify the staged Unix executable's offline runtime")
    );
    assert!(
        step("Verify the staged Unix executable's offline runtime")
            < step("Accept the staged Unix executable from the previous published release")
    );
    let updater =
        steps[step("Accept the staged Unix executable from the previous published release")];
    assert!(updater.contains("GITHUB_TOKEN: ${{ github.token }}"));
    assert!(!updater.contains("CARGO_NET_OFFLINE"));
    // The Windows mise-route step is the job's last step, so this slice holds
    // only that step. Its task re-runs previous-release update acceptance, so
    // it receives only the read-only workflow token for that release listing;
    // the fixture, mise and Kuru children never inherit it.
    assert_eq!(step("Verify the exact staged package"), steps.len() - 1);
    let execution = verifier
        .split("- name: Verify the exact staged package")
        .nth(1)
        .unwrap();
    assert_eq!(execution.matches("GITHUB_TOKEN").count(), 1);
    assert!(execution.contains("GITHUB_TOKEN: ${{ github.token }}"));
    assert!(!execution.contains("GH_TOKEN"));
    assert!(!execution.contains("CARGO_NET_OFFLINE"));

    let publication = job("publish");
    assert!(publication.contains("ref: ${{ needs.bump.outputs.sha }}"));
    assert!(publication.contains("name: ${{ needs.assemble-candidate.outputs.artifact_name }}"));
    assert!(!publication.contains("pattern:"));
    assert_eq!(
        workflow.matches("mise run release:tool -- publish").count(),
        1
    );
    let published = job("verify-published-windows");
    for required in [
        "needs: [plan, bump, publish]",
        "runs-on: windows-latest",
        "contents: read",
        "ref: ${{ needs.bump.outputs.sha }}",
        "RELEASE_VERSION: ${{ needs.plan.outputs.version }}",
        "RELEASE_SHA: ${{ needs.bump.outputs.sha }}",
        "KURU_PUBLISHED_WINDOWS_RECEIPT: ${{ runner.temp }}/published-windows-receipt.json",
        "mise run //packages/kuru-delivery:verify:published-windows",
        "path: ${{ runner.temp }}/published-windows-receipt.json",
        "if-no-files-found: error",
    ] {
        assert!(published.contains(required), "missing {required}");
    }
    assert!(
        workflow.trim_end().ends_with("if-no-files-found: error"),
        "public verification receipt upload must be the final workflow step"
    );
}

const REHEARSAL_BANNER: &str = "REHEARSAL: no commit, tag, release or Pages deployment";
const MODE_INPUT: &str = "        default: auto\n      mode:\n        description: Rehearse without a commit, tag, release or Pages deployment, or publish\n        type: choice\n        options: [rehearsal, publish]\n        default: rehearsal\n";
const RUN_NAME: &str =
    "run-name: ${{ inputs.mode == 'publish' && 'Release' || 'Release rehearsal' }}\n";
const REHEARSAL_CONCURRENCY: &str = "  # A rehearsal never queues behind, or replaces a pending, publication run.\n  group: ${{ inputs.mode == 'publish' && 'release' || 'release-rehearsal' }}\n";
const PUBLISH_GUARD: &str = "    if: inputs.mode == 'publish'\n";
const PUBLISH_STEP_GUARD: &str = "        if: inputs.mode == 'publish'\n";
const REHEARSAL_STEP_GUARD: &str = "        if: inputs.mode == 'rehearsal'\n";
const RELEASE_SOURCE: &str = "inputs.mode == 'publish' && needs.bump.outputs.sha || github.sha";
const PLAN_BANNER_STEP: &str = "      - name: Label this run as a rehearsal\n        if: inputs.mode == 'rehearsal'\n        run: |\n          echo '## REHEARSAL: no commit, tag, release or Pages deployment' >> \"$GITHUB_STEP_SUMMARY\"\n";
const REHEARSAL_PREREQUISITES_STEP: &str = "      - name: Check rehearsal prerequisites\n        if: inputs.mode == 'rehearsal'\n        # A rehearsal never reads the release app key; notes still need theirs.\n        env:\n          RELEASE_APP_ID: ${{ vars.RELEASE_APP_ID }}\n          ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY_COMMUNIQUE }}\n        run: |\n          test -n \"$RELEASE_APP_ID\" || { echo 'Missing RELEASE_APP_ID variable'; exit 1; }\n          test -n \"$ANTHROPIC_API_KEY\" || { echo 'Missing ANTHROPIC_API_KEY_COMMUNIQUE secret'; exit 1; }\n";
const REHEARSAL_STAMP_STEP: &str = "      - name: Stamp the planned version locally for rehearsal\n        if: inputs.mode == 'rehearsal'\n        # The working tree bump would commit; nothing is committed or pushed.\n        shell: bash\n        env:\n          RELEASE_VERSION: ${{ needs.plan.outputs.version }}\n        run: mise run release:tool -- stamp \"$RELEASE_VERSION\"\n";
const NOTES_COMMIT_STEP: &str = "      - name: Commit the planned version locally for rehearsal\n        if: inputs.mode == 'rehearsal'\n        id: local\n        # Notes read an exact clean commit carrying the planned version. This\n        # commit never leaves the runner: the checkout holds no credentials.\n        env:\n          RELEASE_VERSION: ${{ needs.plan.outputs.version }}\n        run: |\n          mise run release:tool -- stamp \"$RELEASE_VERSION\"\n          git -c user.name=kuru-rehearsal -c user.email=rehearsal@invalid -c commit.gpgsign=false -c core.hooksPath=/dev/null commit -q -m \"chore(release): v$RELEASE_VERSION\" -- Cargo.toml Cargo.lock\n          printf 'sha=%s\\n' \"$(git rev-parse HEAD)\" >> \"$GITHUB_OUTPUT\"\n";
const NOTES_SOURCE: &str =
    "inputs.mode == 'publish' && needs.bump.outputs.sha || steps.local.outputs.sha";
const CANDIDATE_BANNER_STEP: &str = "      - name: Label the rehearsal candidate\n        if: inputs.mode == 'rehearsal'\n        env:\n          RELEASE_VERSION: ${{ needs.plan.outputs.version }}\n          CANDIDATE: ${{ steps.artifact.outputs.name }}\n        run: |\n          {\n            echo '## REHEARSAL: no commit, tag, release or Pages deployment'\n            echo\n            echo \"Candidate \\`$CANDIDATE\\` for v$RELEASE_VERSION was assembled from $GITHUB_SHA with the planned version stamped only in each build's working tree. It is a workflow artifact of this run and cannot be published from it.\"\n          } >> \"$GITHUB_STEP_SUMMARY\"\n";

/// Jobs that also run in a rehearsal, with the needs a rehearsal requires to
/// succeed. Each also requires `bump` (and `build` also `verify`) skipped.
const REHEARSAL_JOBS: [(&str, &[&str]); 5] = [
    ("build", &["plan"]),
    ("notes", &["plan"]),
    ("assemble-candidate", &["plan", "build", "notes"]),
    ("verify-staged", &["plan", "assemble-candidate"]),
    ("build-docs", &["assemble-candidate"]),
];
/// Jobs that write, hold write credentials or observe a publication.
const PUBLISH_ONLY_JOBS: [&str; 4] = ["bump", "deploy-docs", "publish", "verify-published-windows"];

/// The exact job-level guard of a job that runs in both modes. The publish
/// branch is literally `success()`, the default GitHub applies to a job
/// without a status function, so publish-mode scheduling is unchanged.
fn rehearsal_guard(job: &str, succeeded: &[&str]) -> String {
    let mut rehearsal = String::from("inputs.mode == 'rehearsal' && !cancelled()");
    let mut needs: Vec<(&str, &str)> = succeeded.iter().map(|need| (*need, "success")).collect();
    needs.push(("bump", "skipped"));
    if job == "build" {
        needs.push(("verify", "skipped"));
    }
    // Keep the workflow's order: plan, bump, then the remaining needs.
    needs.sort_by_key(|(need, _)| match *need {
        "plan" => 0,
        "bump" => 1,
        "verify" => 2,
        _ => 3,
    });
    for (need, result) in needs {
        rehearsal.push_str(&format!(" && needs.{need}.result == '{result}'"));
    }
    format!(
        "    # Publish reduces to the default success() gate; a rehearsal needs bump skipped.\n    if: ${{{{ (inputs.mode == 'publish' && success()) || ({rehearsal}) }}}}\n"
    )
}

fn release_job(workflow: &str, name: &str) -> String {
    let body = workflow
        .split_once(&format!("\n  {name}:\n"))
        .unwrap_or_else(|| panic!("missing release job {name}"))
        .1;
    let mut job = format!("  {name}:\n");
    for line in body.split_inclusive('\n') {
        if line.starts_with("  ") && !line.starts_with("    ") {
            break;
        }
        job.push_str(line);
    }
    job
}

fn replace_exactly(text: &str, from: &str, to: &str, count: usize) -> String {
    assert_eq!(
        text.matches(from).count(),
        count,
        "expected {count} occurrences of {from:?}"
    );
    text.replace(from, to)
}

/// The release workflow as it behaves with `mode: publish`: rehearsal-only
/// steps removed, publish guards and job guards reduced to GitHub's default
/// success gate, and mode-selected expressions replaced by their publish
/// values. Every removal is exact and counted, so any other rehearsal edit
/// leaves `inputs.mode` in the projection and fails.
fn publish_projection(raw: &str) -> String {
    let mut text = replace_exactly(raw, RUN_NAME, "", 1);
    text = replace_exactly(&text, MODE_INPUT, "        default: auto\n", 1);
    text = replace_exactly(&text, REHEARSAL_CONCURRENCY, "  group: release\n", 1);
    text = replace_exactly(&text, PLAN_BANNER_STEP, "", 1);
    text = replace_exactly(&text, REHEARSAL_PREREQUISITES_STEP, "", 1);
    text = replace_exactly(&text, REHEARSAL_STAMP_STEP, "", 2);
    text = replace_exactly(&text, NOTES_COMMIT_STEP, "", 1);
    text = replace_exactly(&text, CANDIDATE_BANNER_STEP, "", 1);
    // The step guard contains the job guard text, so remove it first.
    text = replace_exactly(&text, PUBLISH_STEP_GUARD, "", 1);
    text = replace_exactly(&text, PUBLISH_GUARD, "", PUBLISH_ONLY_JOBS.len());
    for (job, succeeded) in REHEARSAL_JOBS {
        text = replace_exactly(&text, &rehearsal_guard(job, succeeded), "", 1);
    }
    text = replace_exactly(
        &text,
        &format!("${{{{ {RELEASE_SOURCE} }}}}"),
        "${{ needs.bump.outputs.sha }}",
        5,
    );
    text = replace_exactly(
        &text,
        &format!("${{{{ {NOTES_SOURCE} }}}}"),
        "${{ needs.bump.outputs.sha }}",
        1,
    );
    assert!(
        !text.contains("inputs.mode") && !text.contains("REHEARSAL") && !text.contains("rehearsal"),
        "publish projection retains an unrecognized rehearsal edit"
    );
    text
}

#[test]
fn rehearsal_mode_stops_before_every_publication_write() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workflow = fs::read_to_string(root.join(".github/workflows/release.yml")).unwrap();
    // One dispatch entrypoint; the mode input sits beside bump and defaults to
    // the rehearsal, so a real release must select publish explicitly.
    assert!(workflow.contains(&format!(
        "      bump:\n        description: Conventional version bump\n        type: choice\n        options: [auto, major, minor, patch]\n{MODE_INPUT}\n"
    )));
    assert!(workflow.starts_with(&format!(
        "name: Release\n{RUN_NAME}\non:\n  workflow_dispatch:\n"
    )));
    assert!(workflow.contains(&format!(
        "concurrency:\n{REHEARSAL_CONCURRENCY}  cancel-in-progress: false\n"
    )));
    let workflows = fs::read_dir(root.join(".github/workflows"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "yml"))
        .map(|path| fs::read_to_string(path).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        workflows
            .iter()
            .filter(|text| text.contains("release:tool -- publish") || text.contains("deploy-pages"))
            .count(),
        1,
        "rehearsal must not add a second publication workflow"
    );

    // Every input reference is an exact comparison with one of the choices.
    for (index, _) in workflow.match_indices("inputs.mode") {
        let rest = &workflow[index + "inputs.mode".len()..];
        assert!(
            rest.starts_with(" == 'publish'") || rest.starts_with(" == 'rehearsal'"),
            "inexact mode reference: {}",
            &workflow[index..index + 40]
        );
    }

    // Publication writes, their credentials and public verification run only
    // when publish is selected; verify (quality on the version commit) is
    // skipped with bump.
    for name in PUBLISH_ONLY_JOBS {
        let job = release_job(&workflow, name);
        let header = job.split("    runs-on:").next().unwrap();
        assert!(
            header.contains(&format!("\n{PUBLISH_GUARD}")),
            "{name} lacks the publish guard"
        );
        assert_eq!(
            job.matches("inputs.mode").count(),
            1,
            "{name} has extra mode logic"
        );
    }
    let verify = release_job(&workflow, "verify");
    assert!(
        verify
            .starts_with("  verify:\n    needs: bump\n    uses: ./.github/workflows/quality.yml\n")
    );
    assert!(!verify.contains("if:"));

    // Jobs shared with the rehearsal keep their needs and add one exact guard.
    for (name, succeeded) in REHEARSAL_JOBS {
        let job = release_job(&workflow, name);
        let guard = rehearsal_guard(name, succeeded);
        assert_eq!(
            job.matches(&guard).count(),
            1,
            "{name} lost its exact mode guard"
        );
        let needs = job
            .lines()
            .find(|line| line.starts_with("    needs:"))
            .unwrap();
        for need in succeeded.iter().chain(["bump"].iter()) {
            assert!(
                needs.contains(need),
                "{name} guard names a need it lacks: {need}"
            );
        }
        assert!(
            !job.contains("always()"),
            "{name} must not run after failures"
        );
        assert!(!job.contains("continue-on-error:"));
        // Rehearsal checks out the dispatch SHA, which plan uses as its base.
        assert!(
            !job.contains("${{ needs.bump.outputs.sha }}"),
            "{name} reads an unset bump SHA"
        );
        assert!(job.contains(&format!("ref: ${{{{ {RELEASE_SOURCE} }}}}")));
        // No write permission, app token or release-writing credential.
        for forbidden in [
            ": write",
            "RELEASE_APP_PRIVATE_KEY",
            "create-github-app-token",
            "GH_TOKEN",
            "deploy-pages",
            "release:tool -- commit",
            "release:tool -- publish",
        ] {
            assert!(!job.contains(forbidden), "{name} exposes {forbidden}");
        }
    }
    let notes = release_job(&workflow, "notes");
    assert!(notes.contains(&format!("RELEASE_SHA: ${{{{ {NOTES_SOURCE} }}}}")));
    // Notes read a local, unpushed commit of the stamped tree: the tool
    // requires an exact clean commit that already carries the planned version.
    let commit = notes
        .find(NOTES_COMMIT_STEP)
        .expect("notes lacks its rehearsal commit");
    assert!(
        notes
            .find("      - name: Install package-owned release tools\n")
            .unwrap()
            < commit
    );
    assert!(
        commit
            < notes
                .find("      - name: Generate notes without publishing\n")
                .unwrap()
    );
    assert!(notes.contains("persist-credentials: false"));
    assert!(!notes.contains("git push") && !notes.contains(": write"));
    assert_eq!(notes.matches("secrets.").count(), 1);
    assert!(notes.contains("OPENAI_API_KEY: ${{ secrets.ANTHROPIC_API_KEY_COMMUNIQUE }}"));
    for name in ["build", "assemble-candidate", "verify-staged", "build-docs"] {
        assert!(
            !release_job(&workflow, name).contains("secrets."),
            "{name} reads a secret"
        );
    }

    // Plan never reads the release app key in a rehearsal.
    let plan = release_job(&workflow, "plan");
    let steps = workflow_steps(&plan);
    let publish_check = named_step(&steps, "Check release prerequisites");
    assert!(publish_check.starts_with(&format!(
        "name: Check release prerequisites\n{PUBLISH_STEP_GUARD}"
    )));
    assert!(plan.contains(REHEARSAL_PREREQUISITES_STEP));
    assert_eq!(
        publish_check.matches("RELEASE_APP_PRIVATE_KEY").count(),
        plan.matches("RELEASE_APP_PRIVATE_KEY").count()
    );
    assert_eq!(
        workflow.matches("secrets.RELEASE_APP_PRIVATE_KEY").count(),
        2
    );
    assert!(
        release_job(&workflow, "bump")
            .contains("private-key: ${{ secrets.RELEASE_APP_PRIVATE_KEY }}")
    );
    assert!(plan.starts_with("  plan:\n    if: github.ref == 'refs/heads/main'\n"));

    // Rehearsal archives and staged checks see the tree bump would commit.
    for (name, before) in [
        (
            "build",
            "      - name: Build and package native executable\n",
        ),
        ("verify-staged", "      - uses: actions/download-artifact@"),
    ] {
        let job = release_job(&workflow, name);
        let stamp = job
            .find(REHEARSAL_STAMP_STEP)
            .unwrap_or_else(|| panic!("{name} lacks its stamp"));
        assert!(job.find("      - uses: jdx/mise-action@").unwrap() < stamp);
        assert!(stamp < job.find(before).unwrap(), "{name} stamps too late");
    }
    // Only build and verify-staged stamp a working tree; notes commit theirs.
    assert_eq!(workflow.matches(REHEARSAL_STAMP_STEP).count(), 2);
    assert_eq!(workflow.matches(NOTES_COMMIT_STEP).count(), 1);

    // The run summary labels the rehearsal and its candidate.
    assert_eq!(workflow.matches(REHEARSAL_BANNER).count(), 2);
    assert!(plan.contains(PLAN_BANNER_STEP));
    let assembly = release_job(&workflow, "assemble-candidate");
    assert!(assembly.ends_with(&format!("{CANDIDATE_BANNER_STEP}\n")));
    // Rehearsal-only steps are never selected by publish, and vice versa.
    assert_eq!(
        workflow.matches(REHEARSAL_STEP_GUARD).count(),
        6,
        "unexpected rehearsal-only step"
    );

    // Artifacts keep their names in both modes.
    for (name, count) in [
        ("name: release-${{ matrix.target }}\n", 1),
        ("name: release-notes\n", 2),
        ("printf 'name=release-candidate-%s\\n' \"$RUN_ATTEMPT\"", 1),
        ("printf 'name=github-pages-%s\\n' \"$RUN_ATTEMPT\"", 1),
    ] {
        assert_eq!(
            workflow.matches(name).count(),
            count,
            "artifact name changed: {name}"
        );
    }
    for step in workflow
        .split("\n      - ")
        .map(|step| step.split("\n\n").next().unwrap())
        .filter(|step| step.contains("-artifact@"))
    {
        assert!(
            !step.contains("inputs.mode"),
            "artifact step depends on the mode"
        );
    }

    // The projection used by the publish-mode assertions removes only these.
    let projection = publish_projection(&workflow);
    assert!(projection.contains("concurrency:\n  group: release\n  cancel-in-progress: false\n"));
    assert!(projection.starts_with("name: Release\n\non:\n"));
}

#[test]
fn optional_published_windows_diagnostic_keeps_its_native_launcher() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let task = fs::read_to_string(root.join("packages/kuru-delivery/mise.toml")).unwrap();
    let task = task
        .split("[tasks.\"verify:published-windows\"]")
        .nth(1)
        .unwrap();
    assert!(task.contains(
        "run_windows = \"pwsh.exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File packages/kuru-delivery/support/verify-published-windows.ps1\""
    ));
    let script = fs::read_to_string(
        root.join("packages/kuru-delivery/support/verify-published-windows.ps1"),
    )
    .unwrap();
    let resolve = script
        .find("Get-Command mise -CommandType Application")
        .unwrap();
    let clear = script.find("verify-published-windows --mise").unwrap();
    assert!(
        resolve < clear,
        "native mise must be resolved before verifier env clearing"
    );
    assert!(!task.contains("kuru-memory"));
    assert!(!task.contains("kuru-tui"));
}

fn native_workflow() -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    fs::read_to_string(root.join(".github/workflows/native-tests.yml")).unwrap()
}

fn workflow_job<'a>(workflow: &'a str, name: &str, next: &str) -> &'a str {
    workflow
        .split(&format!("\n  {name}:\n"))
        .nth(1)
        .unwrap_or_else(|| panic!("missing native job {name}"))
        .split(&format!("\n  {next}:\n"))
        .next()
        .unwrap()
}

fn workflow_steps(job: &str) -> Vec<String> {
    let steps = format!("\n{}", job.split("    steps:\n").nth(1).unwrap());
    steps
        .split("\n      - ")
        .skip(1)
        .map(str::to_owned)
        .collect()
}

/// The JSON partition list a workflow matrix names for a partition count.
fn partition_list(count: u32) -> String {
    let items: Vec<_> = (1..=count).map(|k| k.to_string()).collect();
    format!("[{}]", items.join(","))
}

fn table_count(os: &str, mode: Mode) -> u32 {
    partition_count(os, mode).unwrap_or_else(|| panic!("no {} partitions for {os}", mode.name()))
}

fn step_env(step: &str) -> String {
    step.split("        env:\n")
        .nth(1)
        .unwrap()
        .split("        run:")
        .next()
        .unwrap()
        .to_owned()
}

fn named_step<'a>(steps: &'a [String], name: &str) -> &'a str {
    steps
        .iter()
        .find(|step| {
            step.starts_with(&format!("name: {name}\n"))
                || step.contains(&format!("\n        name: {name}\n"))
        })
        .unwrap_or_else(|| panic!("missing step {name}"))
}

#[test]
fn native_workflow_partitions_every_os_and_keeps_the_aggregate_fail_closed() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workflow = native_workflow();
    // Every OS runs the same checked partitions and one Ubuntu merge; the
    // package shards, the rebuilding collect job and the unsharded coverage
    // job are gone.
    for retired in [
        "\n  coverage:\n",
        "\n  collect:\n",
        "windows-coverage",
        "coverage:windows:",
        "coverage:collect",
        "mise run coverage",
        "matrix.shard",
        "KURU_COVERAGE_SHARD:",
        "KURU_COVERAGE_PACKAGES",
        "inputs.os != 'windows-latest'",
        "inputs.os == 'windows-latest'",
        "KURU_NATIVE_OS",
        "KURU_NATIVE_WINDOWS_",
    ] {
        assert!(!workflow.contains(retired), "retained {retired}");
    }
    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml")).unwrap();
    let native_tests = ci
        .split("\n  native-tests:\n")
        .nth(1)
        .unwrap()
        .split("\n  native-build:\n")
        .next()
        .unwrap();
    assert!(native_tests.contains("os: [ubuntu-latest, macos-latest, windows-latest]"));
    assert!(native_tests.contains("install: true"));
    for required in [
        "native-gate:",
        "needs: shard\n",
        "needs: [shard, merge, install]",
        "test \"$KURU_NATIVE_SHARDS\" = success",
        "test \"$KURU_NATIVE_REPORT\" = success",
        "test \"$KURU_NATIVE_INSTALL_RESULT\" = success",
        "test \"$KURU_NATIVE_INSTALL_RESULT\" = skipped",
        "KURU_NATIVE_REPORT: ${{ needs.merge.result }}",
        "KURU_COVERAGE_OS: ${{ inputs.os }}",
        "KURU_COVERAGE_TARGET: ${{ runner.temp }}/kuru-coverage-target\n",
        "KURU_COVERAGE_SOURCE: ${{ inputs.ref }}",
        "KURU_COVERAGE_ATTEMPT: ${{ github.run_attempt }}",
        "KURU_COVERAGE_PARTITION: ${{ matrix.partition }}",
        "KURU_COVERAGE_OUTPUT",
        "KURU_COVERAGE_DIAGNOSTICS",
        "KURU_COVERAGE_SEED: ${{ runner.temp }}/kuru-coverage-seed\n",
        "KURU_COVERAGE_HELPER_CACHE: ${{ steps.helper-cache.outputs.cache-hit }}",
        "KURU_COVERAGE_SEED_CACHE: ${{ steps.seed.outputs.cache-hit }}",
        "KURU_COVERAGE_SEED_MATCHED_KEY: ${{ steps.seed.outputs.cache-matched-key }}",
        "KURU_COVERAGE_MODE: instrumented",
        "KURU_COVERAGE_INPUTS",
        "KURU_COVERAGE_REPORT",
        "run: mise run //packages/kuru-delivery:coverage:shard",
        "            mise run //packages/kuru-delivery:coverage:shard\n          KURU_NATIVE_STORE",
        "mise run //packages/kuru-delivery:coverage:merge",
        "if-no-files-found: error",
    ] {
        assert!(workflow.contains(required), "missing {required}");
    }
    let shards = workflow_job(&workflow, "shard", "merge");
    let merge = workflow_job(&workflow, "merge", "install");
    // Partitions run on their own OS; every OS merges on Ubuntu from the
    // partitions' exported LCOV, rebuilding and instrumenting nothing.
    assert!(shards.contains("    runs-on: ${{ inputs.os }}\n"));
    assert!(merge.contains("    runs-on: ubuntu-latest\n"));
    assert!(merge.contains("    name: Coverage merge (${{ inputs.os }})\n"));
    assert!(
        shards
            .contains("    name: Coverage partition (${{ inputs.os }}, ${{ matrix.partition }})\n")
    );
    for forbidden in [
        "cargo-llvm-cov",
        "component add llvm-tools",
        "bundle:prepare",
        "coverage:shard",
        "CARGO_PROFILE_TEST_DEBUG: ${{",
    ] {
        assert!(!merge.contains(forbidden), "merge still uses {forbidden}");
    }
    assert!(merge.contains("install_args: rust\n"));
    for job in [shards, merge] {
        assert!(!job.contains("inputs.install"));
        for moved in ["mise run install", "test:embedded-runtime"] {
            assert!(!job.contains(moved), "coverage still runs {moved}");
        }
    }
    // The matrix and the declared partition count are exactly the table the
    // receipt and merge code enforce, so the workflow cannot drift from it.
    let ubuntu = table_count("ubuntu-latest", Mode::Instrumented);
    let macos = table_count("macos-latest", Mode::Instrumented);
    let windows = table_count("windows-latest", Mode::Instrumented);
    assert_eq!(ubuntu, windows, "the non-macOS branch names one count");
    assert!(shards.contains(&format!(
        "        partition: ${{{{ fromJSON(inputs.os == 'macos-latest' && '{}' || '{}') }}}}\n",
        partition_list(macos),
        partition_list(ubuntu)
    )));
    let count = format!(
        "KURU_COVERAGE_PARTITIONS: ${{{{ inputs.os == 'macos-latest' && '{macos}' || '{ubuntu}' }}}}\n"
    );
    assert_eq!(shards.matches(count.as_str()).count(), 2);
    assert_eq!(merge.matches(count.as_str()).count(), 1);
    // Every partition and the merge share one helper key per OS; exactly
    // partition 1 saves it. The merge keeps its own helper cache.
    assert_eq!(
        workflow
            .matches("shared-key: native-coverage-${{ inputs.os }}\n")
            .count(),
        1
    );
    assert_eq!(
        shards
            .matches(
                "save-if: ${{ matrix.partition == 1 && github.ref == 'refs/heads/main' }}
"
            )
            .count(),
        1
    );
    assert!(merge.contains("shared-key: native-coverage-merge\n"));
    // The seed restores and saves the same path under one pinned action,
    // exports only from main partition 1 on a cache miss, and saves only a
    // staged export, never the consumed remainder of a restored seed.
    let steps = workflow_steps(shards);
    let restore = named_step(&steps, "Restore the instrumented dependency seed");
    for required in [
        "uses: actions/cache/restore@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6.1.0",
        "path: ${{ runner.temp }}/kuru-coverage-seed\n",
        "key: kuru-coverage-seed-v1-${{ inputs.os }}-instrumented-${{ hashFiles('Cargo.lock', 'mise.toml', 'mise.lock') }}",
        "restore-keys: kuru-coverage-seed-v1-${{ inputs.os }}-instrumented-",
    ] {
        assert!(restore.contains(required), "seed restore lost {required}");
    }
    assert!(restore.starts_with("id: seed\n"));
    assert!(named_step(&steps, "Select the dependency seed export").contains(
        "if: github.ref == 'refs/heads/main' && matrix.partition == 1 && steps.seed.outputs.cache-hit != 'true'"
    ));
    let stage = named_step(&steps, "Stage the exported dependency seed");
    assert!(stage.contains("if: env.KURU_COVERAGE_SEED_EXPORT != ''"));
    assert!(stage.contains("rm -rf -- \"$RUNNER_TEMP/kuru-coverage-seed\""));
    let save = named_step(&steps, "Save the instrumented dependency seed");
    for required in [
        "if: env.KURU_COVERAGE_SEED_READY == 'true'",
        "uses: actions/cache/save@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6.1.0",
        "path: ${{ runner.temp }}/kuru-coverage-seed\n",
        "key: ${{ steps.seed.outputs.cache-primary-key }}",
    ] {
        assert!(save.contains(required), "seed save lost {required}");
    }
    let position = |name: &str| {
        steps
            .iter()
            .position(|step| step.starts_with(&format!("name: {name}\n")))
            .unwrap_or_else(|| panic!("missing step {name}"))
    };
    assert!(
        position("Run one checked coverage partition with the OS secret store")
            < position("Stage the exported dependency seed")
    );
    // The merge downloads every partition's attempts in one pattern step.
    assert_eq!(
        workflow
            .matches("actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c")
            .count(),
        1
    );
    assert_eq!(workflow.matches("actions/download-artifact@").count(), 1);
    assert!(merge.contains(
        "          pattern: ${{ inputs.artifact-prefix }}-coverage-${{ inputs.os }}-partition-*\n          merge-multiple: false\n          path: ${{ runner.temp }}/kuru-coverage-inputs\n"
    ));
    // Only a successful partition publishes receipt evidence under the name
    // the merge accepts; failures publish diagnostics under a name it rejects.
    assert_eq!(workflow.matches("if: ${{ !cancelled() }}").count(), 1);
    assert!(!shards.contains("if: ${{ !cancelled() }}"));
    assert_eq!(shards.matches("if: ${{ failure() }}").count(), 1);
    assert!(shards.contains(
        "name: ${{ inputs.artifact-prefix }}-coverage-diagnostics-${{ inputs.os }}-partition-${{ matrix.partition }}-attempt-${{ github.run_attempt }}"
    ));
    assert!(shards.contains(
        "name: ${{ inputs.artifact-prefix }}-coverage-${{ inputs.os }}-partition-${{ matrix.partition }}-attempt-${{ github.run_attempt }}"
    ));
    assert!(merge.contains(
        "name: ${{ inputs.artifact-prefix }}-coverage-${{ inputs.os }}-attempt-${{ github.run_attempt }}"
    ));
    // The job start is recorded before any other step, and the inner deadline
    // derives from the same limit the host enforces.
    assert!(steps[0].starts_with("name: Record the job start for the inner test deadline\n"));
    assert!(steps[0].contains("KURU_COVERAGE_JOB_STARTED=%s"));
    let timeout: Vec<_> = shards
        .lines()
        .filter_map(|line| line.trim().strip_prefix("timeout-minutes: "))
        .collect();
    assert_eq!(timeout, ["45"]);
    let deadlines: Vec<_> = shards
        .lines()
        .filter_map(|line| line.trim().strip_prefix("KURU_COVERAGE_JOB_MINUTES: \""))
        .map(|value| value.strip_suffix('"').unwrap())
        .collect();
    assert_eq!(deadlines, [timeout[0], timeout[0]]);
    // The Linux secret-store leg and the other runners run the same
    // partition under an identical environment, selected by runner.os.
    let other = named_step(&steps, "Run one checked coverage partition");
    let linux = named_step(
        &steps,
        "Run one checked coverage partition with the OS secret store",
    );
    assert!(other.contains("if: runner.os != 'Linux'"));
    assert!(linux.contains("if: runner.os == 'Linux'"));
    assert_eq!(step_env(other), step_env(linux));
    assert!(step_env(other).contains(
        "CARGO_PROFILE_TEST_DEBUG: ${{ runner.os == 'Linux' && '0' || 'line-tables-only' }}\n"
    ));
    // An older successful attempt must not stand in for a partition whose
    // latest attempt failed: the merge runs even after failures, then
    // refuses before any other step unless every partition job succeeded.
    assert!(merge.starts_with("    if: always()\n    needs: shard\n"));
    let merge_steps = workflow_steps(merge);
    assert!(
        merge_steps[0]
            .starts_with("name: Require every coverage partition job to have succeeded\n")
    );
    assert!(merge_steps[0].contains("KURU_COVERAGE_SHARDS_RESULT: ${{ needs.shard.result }}"));
    assert!(merge_steps[0].contains("if [ \"$KURU_COVERAGE_SHARDS_RESULT\" != success ]; then\n"));
    // Every step-level OS selection uses the runner, never the caller's label.
    for line in workflow.lines() {
        if line.starts_with("        if:") {
            assert!(!line.contains("inputs.os"), "step condition {line}");
        }
    }
}

/// Splits workflow text into whole step blocks (the same `\n      - `
/// step-level marker `workflow_steps` uses) and keeps only the ones that
/// contain a `uses: Swatinem/rust-cache@...` key, however it is ordered
/// within the step (first key or after `name:`). Each returned block is
/// exactly one step, bounded by the next step marker or the file end, so a
/// following `- run:` or `- uses:` step can never be read as part of it.
fn rust_cache_blocks(text: &str) -> Vec<&str> {
    text.split("\n      - ")
        .skip(1)
        .filter(|block| block.contains("uses: Swatinem/rust-cache@"))
        .collect()
}

/// Extracts a rust-cache step block's own `save-if:` value (never one merely
/// mentioned in a comment line), trimmed and with a surrounding `${{ }}`
/// template stripped so callers compare against the bare expression.
fn save_if_value(block: &str) -> Option<&str> {
    let raw = block
        .lines()
        .find_map(|line| line.trim_start().strip_prefix("save-if:").map(str::trim))?;
    Some(
        raw.strip_prefix("${{")
            .and_then(|value| value.strip_suffix("}}"))
            .map(str::trim)
            .unwrap_or(raw),
    )
}

/// A `save-if:` value restricts saves to `main` when it is exactly `false`,
/// solely the main-ref check, or that check combined with another condition
/// through `&&`. A bare mention of the ref check without `&&` (or without
/// being the whole expression) does not count, since that shape cannot arise
/// from a deliberate combination.
fn save_if_restricts_to_main(value: &str) -> bool {
    const MAIN_REF: &str = "github.ref == 'refs/heads/main'";
    value == "false" || value == MAIN_REF || (value.contains(MAIN_REF) && value.contains("&&"))
}

#[test]
fn every_rust_cache_step_restricts_saves_to_main() {
    // Every Swatinem/rust-cache step across the CI, quality and native-tests
    // workflows must only save from `main` (or never save at all), so PR
    // runs restore the warm cache instead of thrashing it. A step whose
    // save-if already carries another condition (e.g. one coverage shard)
    // must combine it with the main-ref check rather than drop it.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workflows = root.join(".github/workflows");
    for name in ["ci.yml", "quality.yml", "native-tests.yml"] {
        let text = fs::read_to_string(workflows.join(name)).unwrap();
        let blocks = rust_cache_blocks(&text);
        assert!(!blocks.is_empty(), "{name} has no rust-cache steps");
        for (index, block) in blocks.iter().enumerate() {
            let value = save_if_value(block).unwrap_or_else(|| {
                panic!("{name} rust-cache step {index} has no save-if:\n{block}")
            });
            assert!(
                save_if_restricts_to_main(value),
                "{name} rust-cache step {index} does not restrict saves to main (save-if: {value}):\n{block}"
            );
        }
    }
}

#[test]
fn rust_cache_step_without_save_if_has_no_extracted_value() {
    // A rust-cache step that never sets save-if must be rejected by the
    // regression test above (via the `unwrap_or_else` panic), not silently
    // treated as restricted.
    let yaml = "\
  job:
    steps:
      - uses: Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6 # v2.9.2
        with:
          cache-bin: false
          shared-key: example
      - run: mise run lint:rust
";
    let blocks = rust_cache_blocks(yaml);
    assert_eq!(blocks.len(), 1);
    assert!(save_if_value(blocks[0]).is_none());
}

#[test]
fn rust_cache_step_with_main_ref_only_in_a_comment_is_not_restricted() {
    // The main-ref check must come from the actual save-if value, not merely
    // appear anywhere in the step's text (e.g. an explanatory comment). This
    // also proves the block boundary stops at the next `- run:` step instead
    // of running on into it.
    let yaml = "\
  job:
    steps:
      - uses: Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6 # v2.9.2
        with:
          cache-bin: false
          # Only main saves: github.ref == 'refs/heads/main'
          save-if: true
      - run: mise run lint:rust
";
    let blocks = rust_cache_blocks(yaml);
    assert_eq!(blocks.len(), 1);
    assert!(!blocks[0].contains("mise run lint:rust"));
    let value = save_if_value(blocks[0]).unwrap();
    assert_eq!(value, "true");
    assert!(!save_if_restricts_to_main(value));
}

#[test]
fn rust_cache_step_named_before_uses_still_extracts_its_save_if() {
    // `uses:` need not be the step's first key (e.g. a `name:` line comes
    // first, as in the real windows platform step); the block must still
    // capture the whole step, including a save-if several keys later.
    let yaml = "\
  job:
    steps:
      - name: Cache platform Rust dependencies
        uses: Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6 # v2.9.2
        with:
          cache-bin: false
          shared-key: native-platform-windows
          save-if: ${{ github.ref == 'refs/heads/main' }}
      - name: Test native platform primitives
        run: mise run //packages/kuru-platform:coverage
";
    let blocks = rust_cache_blocks(yaml);
    assert_eq!(blocks.len(), 1);
    let value = save_if_value(blocks[0]).unwrap();
    assert!(save_if_restricts_to_main(value));
}

#[test]
fn arm64_memory_suite_runs_as_gated_uninstrumented_partitions() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml")).unwrap();
    let job = |name: &str, next: &str| {
        ci.split(&format!("\n  {name}:\n"))
            .nth(1)
            .unwrap_or_else(|| panic!("missing CI job {name}"))
            .split(&format!("\n  {next}:\n"))
            .next()
            .unwrap()
            .to_owned()
    };
    // The memory suite moved out of the arm64 build job into partitions.
    let build = job("native-build", "native-memory");
    assert!(!build.contains("//packages/kuru-memory:test"));
    assert!(build.contains("test:embedded-runtime"));
    let partitions = job("native-memory", "native-memory-merge");
    let merge = job("native-memory-merge", "native-platform");
    let count = table_count("ubuntu-24.04-arm", Mode::Uninstrumented);
    assert!(partitions.contains(&format!(
            "        partition: [{}]\n",
            (1..=count)
                .map(|k| k.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    assert!(partitions.contains("    runs-on: ubuntu-24.04-arm\n"));
    assert!(partitions.contains(
        "    name: Native memory partition (ubuntu-24.04-arm, ${{ matrix.partition }})\n"
    ));
    let declared = format!("KURU_COVERAGE_PARTITIONS: \"{count}\"\n");
    assert!(partitions.contains(&declared));
    assert!(merge.contains(&declared));
    for required in [
        "install_args: rust\n",
        "run: mise run //packages/kuru-delivery:setup:test-tools",
        "KURU_COVERAGE_OS: ubuntu-24.04-arm\n",
        "KURU_COVERAGE_PACKAGES: kuru-memory\n",
        "KURU_COVERAGE_TARGET: ${{ runner.temp }}/kuru-coverage-target\n",
        "KURU_COVERAGE_JOB_MINUTES: \"45\"",
        "timeout-minutes: 45\n",
        "key: kuru-coverage-seed-v1-ubuntu-24.04-arm-uninstrumented-",
        "uses: actions/cache/restore@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6.1.0",
        "uses: actions/cache/save@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6.1.0",
        "run: mise run //packages/kuru-delivery:test:partition",
        "name: ci-coverage-ubuntu-24.04-arm-partition-${{ matrix.partition }}-attempt-${{ github.run_attempt }}",
    ] {
        assert!(
            partitions.contains(required),
            "memory partitions lost {required}"
        );
    }
    // Uninstrumented: no coverage tooling, profiles or instrumented task.
    for forbidden in [
        "cargo-llvm-cov",
        "component add llvm-tools",
        "coverage:shard",
        "CARGO_PROFILE_TEST_DEBUG",
    ] {
        assert!(
            !partitions.contains(forbidden),
            "memory partitions use {forbidden}"
        );
    }
    assert!(merge.starts_with("    if: always()\n    needs: native-memory\n"));
    for required in [
        "    runs-on: ubuntu-latest\n",
        "KURU_COVERAGE_SHARDS_RESULT: ${{ needs.native-memory.result }}",
        "pattern: ci-coverage-ubuntu-24.04-arm-partition-*\n",
        "KURU_COVERAGE_MODE: uninstrumented\n",
        // The merge refuses partitions of any other scope.
        "KURU_COVERAGE_PACKAGES: kuru-memory\n",
        "mise run //packages/kuru-delivery:coverage:merge",
    ] {
        assert!(merge.contains(required), "memory merge lost {required}");
    }
    assert!(
        merge
            .split("    steps:\n")
            .nth(1)
            .unwrap()
            .starts_with("      - name: Require every memory partition job to have succeeded\n")
    );
    assert!(ci.contains(
        "needs: [quality, native-tests, native-build, native-memory, native-memory-merge, native-platform]"
    ));
}

#[test]
fn native_workflow_installs_and_accepts_the_previous_release_update_on_every_os() {
    let workflow = native_workflow();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workflows = root.join(".github/workflows");
    for name in ["ci.yml", "native-tests.yml", "release.yml"] {
        let text = fs::read_to_string(workflows.join(name)).unwrap();
        assert!(!text.contains("windows-2025"), "{name} pins windows-2025");
    }
    let ci = fs::read_to_string(workflows.join("ci.yml")).unwrap();
    let native_tests = ci
        .split("\n  native-tests:\n")
        .nth(1)
        .unwrap()
        .split("\n  native-build:\n")
        .next()
        .unwrap();
    assert!(native_tests.contains("install: true"));
    assert!(
        !native_tests.contains("\n    if:"),
        "native tests must run on every CI event"
    );
    for event in [
        "pull_request:",
        "push:\n    branches: [main]",
        "merge_group:",
    ] {
        assert!(ci.contains(event), "CI lost {event}");
    }
    // One install job serves every OS; no Windows-only install job remains.
    assert!(!workflow.contains("windows-install"));
    assert_eq!(workflow.matches("\n  install:\n").count(), 1);
    let install = workflow_job(&workflow, "install", "native-gate");
    let header = install.split("    steps:\n").next().unwrap();
    assert!(header.starts_with("    if: inputs.install\n"));
    for required in ["runs-on: ${{ inputs.os }}", "timeout-minutes: 45"] {
        assert!(header.contains(required), "install job lost {required}");
    }
    assert!(install.contains("shared-key: native-install-${{ inputs.os }}"));
    // Step order is part of the contract: offline installation first, the
    // installed runtime next, and only then the online updater acceptance.
    let steps = workflow_steps(install);
    let step = |name: &str| {
        steps
            .iter()
            .position(|step| step.starts_with(&format!("name: {name}\n")))
            .unwrap_or_else(|| panic!("missing install step {name}"))
    };
    let order = [
        "Reserve space for release and test builds",
        "Fetch locked Cargo inputs before offline verification",
        "Prepare the package-owned bundle",
        "Prepare the package-owned bundle fixtures",
        "Install package-owned test tools",
        "Source install smoke",
        "Native Windows source install smoke",
        "Verify native offline Cargo input failures",
        "Verify installed offline runtime",
        "Inspect native shipping DLL imports",
        "Accept an update from the previous published release",
        "Require unchanged dependency locks",
    ];
    let positions: Vec<_> = order.iter().map(|name| step(name)).collect();
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    for (name, condition) in [
        (
            "Reserve space for release and test builds",
            Some("if: runner.os == 'Linux'"),
        ),
        (
            "Prepare the package-owned bundle fixtures",
            Some("if: runner.os == 'Windows'"),
        ),
        ("Source install smoke", Some("if: runner.os != 'Windows'")),
        (
            "Native Windows source install smoke",
            Some("if: runner.os == 'Windows'"),
        ),
        (
            "Verify native offline Cargo input failures",
            Some("if: runner.os == 'Windows'"),
        ),
        ("Verify installed offline runtime", None),
        (
            "Inspect native shipping DLL imports",
            Some("if: runner.os == 'Windows'"),
        ),
        ("Accept an update from the previous published release", None),
    ] {
        let body = &steps[step(name)];
        match condition {
            Some(condition) => assert!(body.contains(condition), "{name} lost {condition}"),
            None => assert!(!body.contains("if:"), "{name} must run on every OS"),
        }
        assert!(
            !body.contains("inputs.os"),
            "{name} must select by runner.os"
        );
    }
    assert!(steps[step("Native Windows source install smoke")].contains("shell: pwsh"));
    for name in [
        "Source install smoke",
        "Native Windows source install smoke",
    ] {
        let body = &steps[step(name)];
        assert!(body.contains("KURU_INSTALL_DIR: ${{ runner.temp }}/kuru-bin"));
        assert!(body.contains("CARGO_NET_OFFLINE: \"true\""));
        assert!(body.contains("KURU_DOLT_BUNDLE_OFFLINE: \"true\""));
        assert!(body.contains("mise run install"));
    }
    assert!(steps[step("Verify installed offline runtime")].contains(
        "KURU_EMBEDDED_TEST_BINARY: ${{ runner.temp }}/kuru-bin/kuru${{ runner.os == 'Windows' && '.exe' || '' }}"
    ));
    // The updater step is online, receives the token only for release
    // listing, and names the installed shipping executable, never Cargo's
    // target directory, which the Windows build-input check between
    // installation and this step has already rebuilt with all features.
    let updater = &steps[step("Accept an update from the previous published release")];
    for required in [
        "KURU_UPDATE_CANDIDATE_BINARY: ${{ runner.temp }}/kuru-bin/kuru${{ runner.os == 'Windows' && '.exe' || '' }}",
        "GITHUB_TOKEN: ${{ github.token }}",
        "run: mise run //packages/kuru-delivery:test:previous-release-update",
    ] {
        assert!(updater.contains(required), "updater step lost {required}");
    }
    assert!(!updater.contains("CARGO_NET_OFFLINE"));
    assert!(!updater.contains("KURU_DOLT_BUNDLE_OFFLINE"));
    assert!(workflow.contains("permissions:\n  contents: read\n"));
}

#[cfg(unix)]
#[tokio::test]
async fn native_workflow_gate_rejects_incomplete_results() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workflow = native_workflow();
    let gate = workflow
        .split("\n  native-gate:\n")
        .nth(1)
        .unwrap()
        .split("        run: |\n")
        .nth(1)
        .unwrap();
    // The same gate applies to every OS: it never branches on the caller's label.
    assert!(!gate.contains("inputs.os"));
    async fn run(
        root: &Path,
        gate: &str,
        (install, shards, report, install_result): (&str, &str, &str, &str),
    ) -> bool {
        let mut command = command::rooted(root, "bash");
        command
            .args(["-c", gate])
            .env("KURU_NATIVE_INSTALL", install)
            .env("KURU_NATIVE_SHARDS", shards)
            .env("KURU_NATIVE_REPORT", report)
            .env("KURU_NATIVE_INSTALL_RESULT", install_result);
        command::bounded_output(&mut command, Duration::from_secs(5), 4096)
            .await
            .unwrap()
            .status
            .success()
    }

    for accepted in [
        ("true", "success", "success", "success"),
        ("false", "success", "success", "skipped"),
    ] {
        assert!(
            run(&root, gate, accepted).await,
            "gate rejected {accepted:?}"
        );
    }
    for (label, shards, report, install) in [
        ("missing shard", "skipped", "success", "success"),
        ("failed shard", "failure", "success", "success"),
        ("cancelled shard", "cancelled", "success", "success"),
        ("missing report", "success", "skipped", "success"),
        ("failed report", "success", "failure", "success"),
        ("cancelled report", "success", "cancelled", "success"),
        ("missing install", "success", "success", "skipped"),
        ("failed install", "success", "success", "failure"),
        ("cancelled install", "success", "success", "cancelled"),
    ] {
        assert!(
            !run(&root, gate, ("true", shards, report, install)).await,
            "native gate accepted {label}"
        );
    }
    for (label, shards, report, install) in [
        ("failed shard", "failure", "success", "skipped"),
        ("missing report", "success", "skipped", "skipped"),
        ("unrequested install", "success", "success", "success"),
        (
            "failed unrequested install",
            "success",
            "success",
            "failure",
        ),
    ] {
        assert!(
            !run(&root, gate, ("false", shards, report, install)).await,
            "native gate accepted {label} without installation"
        );
    }
}
struct Repo {
    temp: TempDir,
}
impl Repo {
    async fn new() -> Self {
        let this = Self {
            temp: tempfile::tempdir().unwrap(),
        };
        let root = this.root();
        release::git(root, &["init", "-b", "main"]).await.unwrap();
        for (key, value) in [
            ("user.name", "Release fixture"),
            ("user.email", "fixture@example.invalid"),
            ("commit.gpgsign", "false"),
        ] {
            release::git(root, &["config", key, value]).await.unwrap();
        }
        fs::create_dir(root.join("app")).unwrap();
        fs::write(
            root.join("app/Cargo.toml"),
            "[package]\nname = \"kuru\"\nversion.workspace = true\n",
        )
        .unwrap();
        fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = [\"app\"]\n\n[workspace.package]\nversion = \"0.1.0\"\nedition = \"2024\"\n").unwrap();
        fs::write(root.join("Cargo.lock"), "version = 4\n\n[[package]]\nname = \"kuru\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"serde\"\nversion = \"1.0.229\"\nsource = \"registry+https://example.invalid/index\"\nchecksum = \"unchanged\"\n").unwrap();
        fs::write(root.join("cog.toml"), include_str!("../../../cog.toml")).unwrap();
        fs::write(
            root.join("communique.toml"),
            include_str!("../../../communique.toml"),
        )
        .unwrap();
        this.commit("feat: initial peer harness").await;
        this
    }
    fn root(&self) -> &Path {
        self.temp.path()
    }
    async fn commit(&self, message: &str) -> String {
        release::git(self.root(), &["add", "."]).await.unwrap();
        release::git(self.root(), &["commit", "--allow-empty", "-m", message])
            .await
            .unwrap();
        self.head().await
    }
    async fn head(&self) -> String {
        release::git(self.root(), &["rev-parse", "HEAD"])
            .await
            .unwrap()
    }
    async fn tag(&self, tag: &str) {
        release::git(self.root(), &["tag", tag]).await.unwrap();
    }
}
#[tokio::test]
async fn hook_environment_cannot_redirect_rooted_commands() {
    if std::env::var_os(repository_environment::CHILD).is_some() {
        let repo = Repo::new().await;
        fs::write(
            repo.root().join("requested.txt"),
            "belongs to requested root",
        )
        .unwrap();
        repo.commit("fix: update the requested repository").await;
        assert_eq!(
            release::git(repo.root(), &["show", "HEAD:requested.txt"])
                .await
                .unwrap(),
            "belongs to requested root"
        );
        assert_eq!(
            release::git(
                repo.root(),
                &["config", "--default", "clean", "--get", "fixture.hook"]
            )
            .await
            .unwrap(),
            "clean"
        );
        assert_eq!(
            release::git(repo.root(), &["config", "--get", "user.signingkey"])
                .await
                .unwrap(),
            "fixture-signing-key"
        );
        release::run(
            repo.root(),
            env!("CARGO_BIN_EXE_kuru-delivery-fixture"),
            &["check-hook-env"],
        )
        .await
        .unwrap();
        return;
    }
    repository_environment::ForeignRepository::new()
        .assert_test_isolated("hook_environment_cannot_redirect_rooted_commands")
        .await;
}

#[tokio::test]
async fn hook_environment_preserves_versioning_and_private_recovery_index() {
    let foreign = repository_environment::ForeignRepository::new();
    for test in [
        "actual_cog_conventional_history_and_noop_are_read_only",
        "lost_bump_response_reuses_exact_tree_and_parent_even_after_main_advances",
    ] {
        foreign.assert_test_isolated(test).await;
    }
}

#[tokio::test]
async fn actual_cog_conventional_history_and_noop_are_read_only() {
    let repo = Repo::new().await;
    assert_eq!(
        release::compute_version(repo.root(), "auto").await.unwrap(),
        v("0.1.0")
    );
    repo.tag("v0.1.0").await;
    assert!(
        release::compute_version(repo.root(), "auto")
            .await
            .unwrap_err()
            .to_string()
            .contains("no commits")
    );
    repo.commit("fix: preserve isolated memories").await;
    assert_eq!(
        release::compute_version(repo.root(), "auto").await.unwrap(),
        v("0.1.1")
    );
    repo.commit("feat: add peer relationships").await;
    assert_eq!(
        release::compute_version(repo.root(), "auto").await.unwrap(),
        v("0.2.0")
    );
    repo.commit("feat!: replace configuration format").await;
    assert_eq!(
        release::compute_version(repo.root(), "auto").await.unwrap(),
        v("0.2.0")
    );
    for (kind, expected) in [("major", "1.0.0"), ("minor", "0.2.0"), ("patch", "0.1.1")] {
        assert_eq!(
            release::compute_version(repo.root(), kind).await.unwrap(),
            v(expected)
        );
    }
    repo.tag("v1.0.0").await;
    repo.commit("fix!: remove legacy configuration").await;
    assert_eq!(
        release::compute_version(repo.root(), "auto").await.unwrap(),
        v("2.0.0")
    );
    assert_eq!(
        release::git(repo.root(), &["status", "--porcelain"])
            .await
            .unwrap(),
        ""
    );
    assert!(
        release::compute_version(repo.root(), "--help")
            .await
            .is_err()
    );
}
#[tokio::test]
async fn stamps_only_workspace_versions_and_rejects_inconsistent_locks() {
    let repo = Repo::new().await;
    let path = repo.root().join("Cargo.lock");
    let before: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(release::stamp(repo.root(), v("0.2.0")).unwrap().len(), 2);
    assert_eq!(
        release::workspace_version(repo.root(), None).await.unwrap(),
        v("0.2.0")
    );
    let after: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(before["package"][1], after["package"][1]);
    assert_eq!(after["package"][0]["version"].as_str(), Some("0.2.0"));
    assert!(release::stamp(repo.root(), v("0.2.0")).unwrap().is_empty());
    let manifest = fs::read(repo.root().join("Cargo.toml")).unwrap();
    fs::write(
        &path,
        fs::read_to_string(&path)
            .unwrap()
            .replace("version = \"0.2.0\"", "version = \"0.0.1\""),
    )
    .unwrap();
    assert!(
        release::stamp(repo.root(), v("0.3.0"))
            .unwrap_err()
            .to_string()
            .contains("inconsistent")
    );
    assert_eq!(fs::read(repo.root().join("Cargo.toml")).unwrap(), manifest);
    fs::write(path, "version = 4\n").unwrap();
    assert!(
        release::stamp(repo.root(), v("0.3.0"))
            .unwrap_err()
            .to_string()
            .contains("every workspace")
    );
}
#[tokio::test]
async fn plans_initial_release_and_reuses_prepared_or_tagged_heads_without_inputs() {
    let repo = Repo::new().await;
    let head = repo.head().await;
    let initial = release::plan(repo.root(), "auto").await.unwrap();
    assert_eq!(initial.base_sha, head);
    assert_eq!(initial.version, "0.1.0");
    assert!(release::plan(repo.root(), "patch").await.is_err());
    repo.tag("v0.1.0").await;
    for strategy in ["auto", "major", "minor", "patch"] {
        let retry = release::plan(repo.root(), strategy).await.unwrap();
        assert_eq!(retry.base_sha, head);
        assert_eq!(retry.version, "0.1.0");
    }
    repo.commit("feat: add relationships").await;
    let base = repo.head().await;
    let mut plans = Vec::new();
    for strategy in ["auto", "major", "minor", "patch"] {
        plans.push((
            strategy,
            release::plan(repo.root(), strategy).await.unwrap(),
        ));
    }
    release::stamp(repo.root(), v("0.2.0")).unwrap();
    let prepared = repo.commit("chore(release): v0.2.0").await;
    for strategy in ["auto", "major", "minor", "patch"] {
        let retry = release::plan(repo.root(), strategy).await.unwrap();
        assert_eq!(retry.base_sha, prepared);
        assert_eq!(retry.version, "0.2.0");
    }
    repo.tag("v0.2.0").await;
    repo.commit("feat!: later work").await;
    repo.tag("v1.0.0").await;
    release::git(repo.root(), &["checkout", "--detach", &base])
        .await
        .unwrap();
    for (strategy, original) in plans {
        let retry = release::plan(repo.root(), strategy).await.unwrap();
        assert_eq!(retry.base_sha, original.base_sha);
        assert_eq!(retry.version, original.version);
    }
    assert!(release::plan(repo.root(), "invalid").await.is_err());
    fs::write(repo.root().join("untracked"), "dirty").unwrap();
    assert!(
        release::plan(repo.root(), "auto")
            .await
            .unwrap_err()
            .to_string()
            .contains("clean checkout")
    );
    for invalid in ["1.2", "01.2.3", "1.0.0\nx=y", "1.0.0;id", "1.0.0-rc.1", ""] {
        assert!(invalid.parse::<Version>().is_err());
    }
    for invalid in ["HEAD", "abcd", "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"] {
        assert!(release::checked_sha(invalid).is_err());
    }
}

#[tokio::test]
async fn real_cli_calculates_plans_and_stamps_without_remote_credentials() {
    let repo = Repo::new().await;
    let binary = env!("CARGO_BIN_EXE_kuru-release");
    let output = kuru_delivery::command::Command::new(binary)
        .arg("--root")
        .arg(repo.root())
        .args(["version", "auto"])
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), "0.1.0");
    let outputs = repo.root().join("outputs");
    let output = kuru_delivery::command::Command::new(binary)
        .arg("--root")
        .arg(repo.root())
        .args(["plan", "--bump", "auto"])
        .env("GITHUB_OUTPUT", &outputs)
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    assert!(
        fs::read_to_string(outputs)
            .unwrap()
            .contains("version=0.1.0\n")
    );
    let output = kuru_delivery::command::Command::new(binary)
        .arg("--root")
        .arg(repo.root())
        .args(["stamp", "0.2.0"])
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        release::workspace_version(repo.root(), None).await.unwrap(),
        v("0.2.0")
    );
    let output = kuru_delivery::command::Command::new(binary)
        .args(["commit", "--version", "0.2.0", "--expected-sha", A])
        .env_remove("GH_REPO")
        .env_remove("GH_TOKEN")
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("GH_REPO is required"));
    let output = kuru_delivery::command::Command::new(binary)
        .args([
            "publish",
            "--version",
            "0.2.0",
            "--sha",
            A,
            "--notes",
            "missing",
        ])
        .env("GH_REPO", "fixture/kuru")
        .env_remove("GH_TOKEN")
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("GH_TOKEN is required"));
    let output = kuru_delivery::command::Command::new(binary)
        .args(["stamp", "1.0.0;id"])
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
}

#[tokio::test]
async fn real_cli_assembles_a_candidate_without_github_credentials() {
    let archives = Archives::new();
    let output = kuru_delivery::command::Command::new(env!("CARGO_BIN_EXE_kuru-release"))
        .args([
            "assemble",
            "--version",
            "0.1.0",
            "--directory",
            archives.directory.to_str().unwrap(),
            "--notes",
            archives.notes.to_str().unwrap(),
        ])
        .env_remove("GH_REPO")
        .env_remove("GH_TOKEN")
        .output()
        .await
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        format!(
            "Assembled {} release candidate checks",
            release::TARGETS.len() * 2 + 1
        )
    );
    assert_eq!(
        fs::read_to_string(archives.directory.join("SHA256SUMS"))
            .unwrap()
            .lines()
            .count(),
        release::TARGETS.len() * 2
    );
}

#[derive(Default)]
struct Remote {
    head: String,
    tag: Option<Value>,
    tag_object: Option<Value>,
    release: Option<Value>,
    calls: Vec<(Method, String, Value)>,
    uploads: Vec<String>,
    fail_upload: Option<usize>,
    corrupt_upload: bool,
    malformed: bool,
    prepared_commit: Option<Value>,
    comparison: Option<Value>,
    lose_commit_response: bool,
    manifest: Vec<u8>,
    manifest_redirect: Option<String>,
    lose_publish_response: bool,
}
struct Server {
    api: GitHub,
    state: Arc<Mutex<Remote>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn new(head: &str) -> Self {
        let state = Arc::new(Mutex::new(Remote {
            head: head.into(),
            ..Remote::default()
        }));
        let app = Router::new().fallback(handler).with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            api: GitHub::with_endpoints("fixture/kuru", "fixture-token", &endpoint, &endpoint)
                .unwrap(),
            state,
            task,
        }
    }
}
fn response(status: StatusCode, value: Value) -> Response {
    (status, axum::Json(value)).into_response()
}
async fn handler(State(shared): State<Arc<Mutex<Remote>>>, request: Request) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let query = request.uri().query().unwrap_or_default().to_owned();
    assert_eq!(
        request.headers().get("authorization").unwrap(),
        "Bearer fixture-token"
    );
    let bytes = to_bytes(request.into_body(), 300 * 1024 * 1024)
        .await
        .unwrap();
    let payload = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let mut state = shared.lock().unwrap();
    state
        .calls
        .push((method.clone(), path.clone(), payload.clone()));
    if state.malformed {
        return (StatusCode::OK, "invalid-json").into_response();
    }
    let suffix = path.strip_prefix("/repos/fixture/kuru/").unwrap_or(&path);
    let value = match (method.clone(), suffix) {
        (Method::POST, "/graphql") => {
            if payload["variables"]["input"]["expectedHeadOid"].as_str() != Some(&state.head) {
                return response(
                    StatusCode::OK,
                    json!({"errors":[{"message":"expected head changed; private context"}]}),
                );
            }
            let sha = state
                .prepared_commit
                .as_ref()
                .map(|commit| commit["sha"].as_str().unwrap())
                .unwrap_or(B)
                .to_owned();
            state.head = sha.clone();
            if state.lose_commit_response {
                return response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    json!({"message":"response lost after commit"}),
                );
            }
            json!({"data":{"createCommitOnBranch":{"commit":{"oid":sha}}}})
        }
        (Method::GET, "commits/main") => json!({"sha":state.head}),
        (Method::GET, value) if value.starts_with("compare/") => {
            assert_eq!(query, "per_page=1");
            state
                .comparison
                .clone()
                .unwrap_or(json!({"status":"diverged"}))
        }
        (Method::GET, value) if value.starts_with("git/ref/tags/") => {
            if let Some(tag) = &state.tag {
                json!({"object":tag})
            } else {
                return response(StatusCode::NOT_FOUND, json!({"message":"missing"}));
            }
        }
        (Method::POST, "git/tags") => {
            state.tag_object = Some(json!({"type":"commit","sha":payload["object"]}));
            json!({"sha":C})
        }
        (Method::POST, "git/refs") => {
            state.tag = Some(json!({"type":"tag","sha":payload["sha"]}));
            json!({"object":state.tag})
        }
        (Method::GET, value) if value.starts_with("git/tags/") => {
            json!({"object":state.tag_object})
        }
        (Method::GET, "releases") => state
            .release
            .as_ref()
            .map(|r| json!([r]))
            .unwrap_or(json!([])),
        (Method::POST, "releases") => {
            let mut value = payload;
            value["id"] = json!(17);
            value["assets"] = json!([]);
            value["html_url"] = json!("https://example.invalid/release");
            state.release = Some(value.clone());
            value
        }
        (Method::GET, "releases/17") => state.release.clone().unwrap(),
        (Method::POST, "releases/17/assets") => {
            if state.fail_upload == Some(state.uploads.len()) {
                return response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    json!({"message":"fixture interrupted"}),
                );
            }
            let name = url::form_urlencoded::parse(query.as_bytes())
                .find(|(key, _)| key == "name")
                .unwrap()
                .1
                .into_owned();
            state.uploads.push(name.clone());
            let digest = if state.corrupt_upload {
                "0".repeat(64)
            } else {
                digest(&bytes)
            };
            let id = state.uploads.len() as u64;
            if name == "SHA256SUMS" {
                state.manifest = bytes.to_vec();
            }
            let asset = json!({"id":id,"name":name,"digest":format!("sha256:{digest}")});
            state.release.as_mut().unwrap()["assets"]
                .as_array_mut()
                .unwrap()
                .push(asset.clone());
            asset
        }
        (Method::GET, value) if value.starts_with("releases/assets/") => {
            if let Some(location) = &state.manifest_redirect {
                return (StatusCode::FOUND, [("location", location.clone())]).into_response();
            }
            return (StatusCode::OK, state.manifest.clone()).into_response();
        }
        (Method::PATCH, "releases/17") => {
            assert_eq!(
                payload["make_latest"], "legacy",
                "recovery must not force an older draft to become latest"
            );
            let release = state.release.as_mut().unwrap();
            release["draft"] = payload["draft"].clone();
            let result = release.clone();
            if state.lose_publish_response {
                return response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    json!({"message":"response lost after publication"}),
                );
            }
            result
        }
        _ => {
            return response(
                StatusCode::BAD_REQUEST,
                json!({"message":"unexpected request"}),
            );
        }
    };
    response(StatusCode::OK, value)
}
#[tokio::test]
async fn signed_api_commit_uses_expected_head_and_only_scoped_payloads() {
    let repo = Repo::new().await;
    let head = repo.head().await;
    let server = Server::new(&head).await;
    assert_eq!(
        release::commit_version(repo.root(), &server.api, &head, v("0.1.0"))
            .await
            .unwrap(),
        head
    );
    assert!(
        server
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|(method, _, _)| method == Method::GET)
    );
    release::stamp(repo.root(), v("0.2.0")).unwrap();
    assert_eq!(
        release::commit_version(repo.root(), &server.api, &head, v("0.2.0"))
            .await
            .unwrap(),
        B
    );
    let payload = server
        .state
        .lock()
        .unwrap()
        .calls
        .iter()
        .find(|(_, path, _)| path == "/graphql")
        .unwrap()
        .2
        .clone();
    let input = &payload["variables"]["input"];
    assert_eq!(input["expectedHeadOid"], head);
    assert_eq!(input["branch"]["branchName"], "main");
    assert_eq!(input["message"]["headline"], "chore(release): v0.2.0");
    let files = input["fileChanges"]["additions"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    let manifest = files.iter().find(|f| f["path"] == "Cargo.toml").unwrap();
    let data: toml::Value = toml::from_str(
        &String::from_utf8(
            STANDARD
                .decode(manifest["contents"].as_str().unwrap())
                .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        data["workspace"]["package"]["version"].as_str(),
        Some("0.2.0")
    );
    server.state.lock().unwrap().head = C.into();
    let error = release::commit_version(repo.root(), &server.api, &head, v("0.2.0"))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("diverged"));
    assert!(!error.contains("private context"));
    fs::write(repo.root().join("cog.toml"), "# unrelated change\n").unwrap();
    let requests_before = server.state.lock().unwrap().calls.len();
    assert_eq!(
        release::commit_version(repo.root(), &server.api, &head, v("0.2.0"))
            .await
            .unwrap_err()
            .to_string(),
        "release stamp changed unrelated files: cog.toml",
    );
    assert_eq!(server.state.lock().unwrap().calls.len(), requests_before);
}
async fn comparison(repo: &Repo, base: &str, head: &str) -> Value {
    let commits = release::git(
        repo.root(),
        &["rev-list", "--reverse", &format!("{base}..{head}")],
    )
    .await
    .unwrap();
    let first = commits.lines().next().unwrap();
    let parents = release::git(repo.root(), &["show", "-s", "--format=%P", first])
        .await
        .unwrap();
    let tree = release::git(repo.root(), &["rev-parse", &format!("{first}^{{tree}}")])
        .await
        .unwrap();
    let message = release::git(repo.root(), &["show", "-s", "--format=%B", first])
        .await
        .unwrap();
    json!({"status":"ahead", "merge_base_commit":{"sha":base}, "commits":[{
        "sha":first, "parents":parents.split_whitespace().map(|sha| json!({"sha":sha})).collect::<Vec<_>>(),
        "commit":{"message":message, "tree":{"sha":tree}}
    }]})
}
#[tokio::test]
async fn lost_bump_response_reuses_exact_tree_and_parent_even_after_main_advances() {
    let repo = Repo::new().await;
    let base = repo.head().await;
    release::stamp(repo.root(), v("0.2.0")).unwrap();
    let prepared = repo.commit("chore(release): v0.2.0").await;
    let expected = comparison(&repo, &base, &prepared).await;
    fs::write(repo.root().join("later.txt"), "not part of this release").unwrap();
    let later = repo.commit("feat: later independent work").await;
    let later_comparison = comparison(&repo, &base, &later).await;
    release::git(repo.root(), &["checkout", "--detach", &base])
        .await
        .unwrap();
    release::stamp(repo.root(), v("0.2.0")).unwrap();
    let original_index = fs::read(repo.root().join(".git/index")).unwrap();
    let server = Server::new(&base).await;
    {
        let mut remote = server.state.lock().unwrap();
        remote.prepared_commit = Some(expected["commits"][0].clone());
        remote.comparison = Some(expected.clone());
        remote.lose_commit_response = true;
    }
    assert!(
        release::commit_version(repo.root(), &server.api, &base, v("0.2.0"))
            .await
            .is_err()
    );
    assert_eq!(server.state.lock().unwrap().head, prepared);
    assert_eq!(
        release::commit_version(repo.root(), &server.api, &base, v("0.2.0"))
            .await
            .unwrap(),
        prepared
    );
    {
        let mut remote = server.state.lock().unwrap();
        remote.head = later;
        remote.comparison = Some(later_comparison);
    }
    assert_eq!(
        release::commit_version(repo.root(), &server.api, &base, v("0.2.0"))
            .await
            .unwrap(),
        prepared
    );
    assert_eq!(
        fs::read(repo.root().join(".git/index")).unwrap(),
        original_index
    );
    assert_eq!(
        server
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|(method, _, _)| method == Method::POST)
            .count(),
        1
    );

    // A title alone, or even identical contents with the wrong parent, is not
    // evidence that an intervening commit is the prepared release.
    for field in ["tree", "parent", "message", "merge", "diverged"] {
        let mut bad = expected.clone();
        match field {
            "tree" => bad["commits"][0]["commit"]["tree"]["sha"] = json!(C),
            "parent" => bad["commits"][0]["parents"][0]["sha"] = json!(C),
            "message" => bad["commits"][0]["commit"]["message"] = json!("feat: unrelated work"),
            "merge" => bad["commits"][0]["parents"]
                .as_array_mut()
                .unwrap()
                .push(json!({"sha":C})),
            _ => bad["status"] = json!("diverged"),
        }
        server.state.lock().unwrap().comparison = Some(bad);
        assert!(
            release::commit_version(repo.root(), &server.api, &base, v("0.2.0"))
                .await
                .is_err(),
            "accepted {field}"
        );
    }
    assert_eq!(
        server
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|(method, _, _)| method == Method::POST)
            .count(),
        1
    );
}
#[tokio::test]
async fn unchanged_stamp_keeps_original_ancestor_without_including_new_main_work() {
    let repo = Repo::new().await;
    let base = repo.head().await;
    fs::write(repo.root().join("later.txt"), "later").unwrap();
    let later = repo.commit("fix: later work").await;
    let ahead = comparison(&repo, &base, &later).await;
    release::git(repo.root(), &["checkout", "--detach", &base])
        .await
        .unwrap();
    let server = Server::new(&later).await;
    server.state.lock().unwrap().comparison = Some(ahead);
    assert_eq!(
        release::commit_version(repo.root(), &server.api, &base, v("0.1.0"))
            .await
            .unwrap(),
        base
    );
    assert!(
        server
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|(method, _, _)| method == Method::GET)
    );
    server.state.lock().unwrap().comparison.as_mut().unwrap()["merge_base_commit"]["sha"] =
        json!(C);
    assert!(
        release::commit_version(repo.root(), &server.api, &base, v("0.1.0"))
            .await
            .is_err()
    );
}
struct Archives {
    temp: TempDir,
    directory: PathBuf,
    notes: PathBuf,
}
impl Archives {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("dist");
        fs::create_dir(&directory).unwrap();
        // The candidate verifier decodes every exact core and support inventory.
        let binary = temp.path().join("fixture-kuru");
        fs::write(&binary, b"#!/bin/sh\nexit 0\n").unwrap();
        make_executable(&File::open(&binary).unwrap()).unwrap();
        let generated = temp.path().join("generated");
        fs::create_dir_all(generated.join("completions")).unwrap();
        fs::create_dir_all(generated.join("man")).unwrap();
        for name in shell_support::NAMES {
            fs::write(generated.join(name), format!("fixture {name}\n")).unwrap();
        }
        for target in release::TARGETS {
            kuru_delivery::archive::package(&binary, target, "0.1.0", &directory).unwrap();
            shell_support::package(&generated, target, "0.1.0", &directory).unwrap();
        }
        let notes = temp.path().join("notes.md");
        fs::write(&notes, "# Kuru 0.1.0\n\nPersistent peer conversations.\n").unwrap();
        release::assemble(&directory, v("0.1.0"), &notes).unwrap();
        Self {
            temp,
            directory,
            notes,
        }
    }
    async fn publish(&self, api: &GitHub) -> anyhow::Result<String> {
        release::publish(api, &self.directory, v("0.1.0"), A, &self.notes).await
    }
}
#[tokio::test]
async fn interrupted_draft_resumes_missing_assets_then_publishes_exact_commit() {
    let archives = Archives::new();
    let server = Server::new(A).await;
    server.state.lock().unwrap().fail_upload = Some(2);
    assert!(archives.publish(&server.api).await.is_err());
    {
        let remote = server.state.lock().unwrap();
        assert_eq!(remote.release.as_ref().unwrap()["draft"], true);
        assert!(
            remote
                .calls
                .iter()
                .all(|(method, _, _)| method != Method::PATCH)
        );
    }
    server.state.lock().unwrap().fail_upload = None;
    assert_eq!(
        archives.publish(&server.api).await.unwrap(),
        "https://example.invalid/release"
    );
    assert_eq!(
        release::tag_commit(&server.api, v("0.1.0"))
            .await
            .unwrap()
            .as_deref(),
        Some(A)
    );
    {
        let remote = server.state.lock().unwrap();
        assert_eq!(remote.uploads.len(), release::TARGETS.len() * 2 + 1);
        assert_eq!(remote.release.as_ref().unwrap()["draft"], false);
        assert!(
            remote.release.as_ref().unwrap()["body"]
                .as_str()
                .unwrap()
                .contains(A)
        );
    }
    let snapshot = server.state.lock().unwrap().release.clone();
    server.state.lock().unwrap().calls.clear();
    assert_eq!(
        archives.publish(&server.api).await.unwrap(),
        "https://example.invalid/release"
    );
    assert_eq!(server.state.lock().unwrap().release, snapshot);
    assert!(
        server
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|(method, _, _)| method == Method::GET)
    );
    assert_eq!(
        fs::read_to_string(archives.directory.join("SHA256SUMS"))
            .unwrap()
            .lines()
            .count(),
        release::TARGETS.len() * 2
    );
}
#[tokio::test]
async fn lost_publication_response_recovers_from_remote_checksums_without_rebuilding() {
    let archives = Archives::new();
    let server = Server::new(A).await;
    server.state.lock().unwrap().lose_publish_response = true;
    assert!(archives.publish(&server.api).await.is_err());
    assert_eq!(
        server.state.lock().unwrap().release.as_ref().unwrap()["draft"],
        false
    );
    fs::remove_dir_all(&archives.directory).unwrap();
    fs::remove_file(&archives.notes).unwrap();
    server.state.lock().unwrap().calls.clear();
    assert_eq!(
        archives.publish(&server.api).await.unwrap(),
        "https://example.invalid/release"
    );
    assert!(
        server
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|(method, _, _)| method == Method::GET)
    );
}
#[tokio::test]
async fn published_recovery_rejects_incomplete_or_mismatched_remote_content() {
    let archives = Archives::new();
    let server = Server::new(A).await;
    archives.publish(&server.api).await.unwrap();
    let original = server.state.lock().unwrap().release.clone().unwrap();
    let original_manifest = server.state.lock().unwrap().manifest.clone();
    for invalid in [
        "asset",
        "digest",
        "marker",
        "manifest",
        "oversize",
        "redirect",
        "missing-tag",
    ] {
        {
            let mut remote = server.state.lock().unwrap();
            remote.release = Some(original.clone());
            remote.manifest = original_manifest.clone();
            remote.calls.clear();
            remote.manifest_redirect = None;
            match invalid {
                "asset" => {
                    remote.release.as_mut().unwrap()["assets"]
                        .as_array_mut()
                        .unwrap()
                        .pop();
                }
                "digest" => {
                    remote.release.as_mut().unwrap()["assets"][1]["digest"] =
                        json!(format!("sha256:{}", "0".repeat(64)))
                }
                "marker" => remote.release.as_mut().unwrap()["body"] = json!("unrelated release"),
                "manifest" => remote.manifest = b"inconsistent checksum data".to_vec(),
                "oversize" => remote.manifest = vec![b'x'; 4097],
                "redirect" => {
                    remote.manifest_redirect = Some("http://example.invalid/unsafe".into())
                }
                _ => remote.tag = None,
            }
        }
        assert!(
            archives.publish(&server.api).await.is_err(),
            "accepted {invalid}"
        );
        assert!(
            server
                .state
                .lock()
                .unwrap()
                .calls
                .iter()
                .all(|(method, _, _)| method == Method::GET)
        );
    }
}
#[tokio::test]
async fn missing_corrupt_assets_and_empty_notes_prevent_all_remote_writes() {
    let archives = Archives::new();
    let server = Server::new(A).await;
    let support_name = shell_support::archive_name("0.1.0", release::TARGETS[0]).unwrap();
    let support_path = archives.directory.join(&support_name);
    let original_support = fs::read(&support_path).unwrap();
    fs::remove_file(&support_path).unwrap();
    assert!(
        release::assemble(&archives.directory, v("0.1.0"), &archives.notes)
            .unwrap_err()
            .to_string()
            .contains("a core and a paired shell support archive for every release target")
    );
    fs::write(&support_path, b"not a support envelope").unwrap();
    fs::write(
        archives.directory.join(format!("{support_name}.sha256")),
        format!("{}  {support_name}\n", digest(b"not a support envelope")),
    )
    .unwrap();
    assert!(
        release::assemble(&archives.directory, v("0.1.0"), &archives.notes).is_err(),
        "candidate accepted a checksummed but invalid support envelope"
    );
    fs::write(&support_path, &original_support).unwrap();
    fs::write(
        archives.directory.join(format!("{support_name}.sha256")),
        format!("{}  {support_name}\n", digest(&original_support)),
    )
    .unwrap();
    let name = kuru_delivery::archive::archive_name("0.1.0", release::TARGETS[0]).unwrap();
    let path = archives.directory.join(name);
    let original = fs::read(&path).unwrap();
    fs::remove_file(&path).unwrap();
    assert!(
        release::assemble(&archives.directory, v("0.1.0"), &archives.notes)
            .unwrap_err()
            .to_string()
            .contains("a core and a paired shell support archive for every release target")
    );
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("a core and a paired shell support archive for every release target")
    );
    fs::write(&path, b"corrupt").unwrap();
    assert!(
        release::assemble(&archives.directory, v("0.1.0"), &archives.notes)
            .unwrap_err()
            .to_string()
            .contains("checksum mismatch")
    );
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("checksum mismatch")
    );
    fs::write(&path, &original).unwrap();
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(
        release::assemble(&archives.directory, v("0.1.0"), &archives.notes)
            .unwrap_err()
            .to_string()
            .contains("unexpected or nonregular")
    );
    fs::remove_dir(&path).unwrap();
    fs::write(&path, &original).unwrap();
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(256 * 1024 * 1024 + 1)
        .unwrap();
    assert!(
        release::assemble(&archives.directory, v("0.1.0"), &archives.notes)
            .unwrap_err()
            .to_string()
            .contains("bounded regular")
    );
    fs::write(path, original).unwrap();
    fs::write(archives.directory.join("unexpected.sha256"), "unexpected\n").unwrap();
    assert!(
        release::assemble(&archives.directory, v("0.1.0"), &archives.notes)
            .unwrap_err()
            .to_string()
            .contains("unexpected or nonregular")
    );
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("unexpected or nonregular")
    );
    fs::remove_file(archives.directory.join("unexpected.sha256")).unwrap();
    fs::write(archives.directory.join("SHA256SUMS"), "stale\n").unwrap();
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("candidate checksum manifest")
    );
    release::assemble(&archives.directory, v("0.1.0"), &archives.notes).unwrap();
    fs::write(&archives.notes, "").unwrap();
    assert!(
        release::assemble(&archives.directory, v("0.1.0"), &archives.notes)
            .unwrap_err()
            .to_string()
            .contains("nonempty")
    );
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("nonempty")
    );
    assert!(
        server
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|(method, _, _)| method == Method::GET)
    );
}
#[tokio::test]
async fn immutable_tag_conflicts_and_unowned_or_corrupt_drafts_are_rejected() {
    let archives = Archives::new();
    let server = Server::new(A).await;
    server.state.lock().unwrap().tag = Some(json!({"type":"commit","sha":B}));
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("different commit")
    );
    assert!(
        server
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|(method, _, _)| method == Method::GET)
    );
    server.state.lock().unwrap().tag = None;
    server.state.lock().unwrap().fail_upload = Some(1);
    assert!(archives.publish(&server.api).await.is_err());
    server.state.lock().unwrap().release.as_mut().unwrap()["assets"][0]["digest"] = json!("bad");
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("differs")
    );
    server.state.lock().unwrap().release.as_mut().unwrap()["body"] = json!("another draft");
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("does not belong")
    );
}
#[tokio::test]
async fn bad_api_shapes_and_final_uploaded_digest_never_publish() {
    let archives = Archives::new();
    let server = Server::new(A).await;
    server.state.lock().unwrap().corrupt_upload = true;
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("differs")
    );
    assert!(
        server
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|(method, _, _)| method != Method::PATCH)
    );
    server.state.lock().unwrap().malformed = true;
    assert!(release::tag_commit(&server.api, v("0.1.0")).await.is_err());
    server.state.lock().unwrap().malformed = false;
    server.state.lock().unwrap().tag = Some(json!({"type":"tree","sha":C}));
    assert!(
        release::tag_commit(&server.api, v("0.1.0"))
            .await
            .unwrap_err()
            .to_string()
            .contains("resolve")
    );
    server.state.lock().unwrap().tag = Some(json!({"type":"tag","sha":C}));
    server.state.lock().unwrap().tag_object = Some(json!({"type":"tag","sha":C}));
    assert!(
        release::tag_commit(&server.api, v("0.1.0"))
            .await
            .unwrap_err()
            .to_string()
            .contains("resolve")
    );
    assert!(GitHub::new("bad/repo/path", "fixture").is_err());
    assert!(
        GitHub::with_endpoints(
            "fixture/kuru",
            "fixture",
            "http://example.invalid/",
            "http://example.invalid/"
        )
        .is_err()
    );
    assert!(GitHub::new("fixture/kuru", "").is_err());
    assert!(archives.temp.path().exists());
}
