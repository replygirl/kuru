#![cfg(feature = "tooling")]

use axum::{
    Router,
    extract::{Request, State},
    http::{Method, StatusCode},
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use kuru_delivery::{
    archive, homebrew,
    release::{GitHub, Version},
    shell_support, targets,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

fn version(text: &str) -> Version {
    text.parse().unwrap()
}
fn hashes() -> BTreeMap<String, String> {
    let mut hashes = BTreeMap::from([("SHA256SUMS".to_owned(), "a".repeat(64))]);
    for target in targets::CATALOG {
        hashes.insert(
            archive::archive_name("0.12.0", target.triple).unwrap(),
            "b".repeat(64),
        );
        hashes.insert(
            shell_support::archive_name("0.12.0", target.triple).unwrap(),
            "c".repeat(64),
        );
    }
    hashes
}

#[test]
fn generation_requires_complete_release_and_exact_paired_supported_assets() {
    let hashes = hashes();
    let retained = hashes.clone();
    let formula =
        homebrew::generate(version("0.12.0"), homebrew::SOURCE_REPOSITORY, &hashes).unwrap();
    assert_eq!(hashes, retained);
    for target in targets::CATALOG
        .into_iter()
        .filter(|target| target.os != "windows")
    {
        assert!(formula.contains(&archive::archive_name("0.12.0", target.triple).unwrap()));
        assert!(formula.contains(&shell_support::archive_name("0.12.0", target.triple).unwrap()));
    }
    assert_eq!(formula.matches("resource \"shell-support\"").count(), 3);
    assert_eq!(formula.matches("sha256 \"").count(), 6);
    assert!(formula.contains("skip_clean \"bin/kuru\""));
    assert!(formula.contains("depends_on arch: :arm64"));
    for forbidden in [
        "x86_64-apple-darwin",
        "windows",
        "depends_on \"dolt\"",
        "cargo",
        "generate_completions_from_executable",
        "latest",
        "bottle do",
    ] {
        assert!(!formula.contains(forbidden), "unexpected {forbidden}");
    }
    for name in hashes.keys() {
        let mut missing = hashes.clone();
        missing.remove(name);
        assert!(
            homebrew::generate(version("0.12.0"), homebrew::SOURCE_REPOSITORY, &missing).is_err()
        );
    }
    for invalid in ["", "a", &"A".repeat(64), &"g".repeat(64), &"a".repeat(65)] {
        let mut malformed = hashes.clone();
        malformed.insert("SHA256SUMS".into(), invalid.into());
        assert!(
            homebrew::generate(version("0.12.0"), homebrew::SOURCE_REPOSITORY, &malformed).is_err()
        );
    }
    let mut extra = hashes.clone();
    extra.insert("unexpected".into(), "a".repeat(64));
    assert!(homebrew::generate(version("0.12.0"), homebrew::SOURCE_REPOSITORY, &extra).is_err());
    assert!(homebrew::generate(version("0.12.0"), "arbitrary/repo", &hashes).is_err());
}

#[derive(Default)]
struct Fixture {
    current: Option<Value>,
    puts: Vec<Value>,
    conflict: bool,
    incomplete_receipt: bool,
    lose_response_after_write: bool,
}
type Shared = Arc<Mutex<Fixture>>;

async fn handle(State(state): State<Shared>, request: Request) -> Response {
    assert_eq!(request.headers()["authorization"], "Bearer fake-tap-token");
    assert_eq!(
        request.uri().path(),
        "/repos/replygirl/homebrew-kuru/contents/Formula/kuru.rb"
    );
    if request.method() == Method::GET {
        assert_eq!(request.uri().query(), Some("ref=main"));
        return match state.lock().unwrap().current.clone() {
            Some(value) => axum::Json(value).into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        };
    }
    assert_eq!(request.method(), Method::PUT);
    let body = axum::body::to_bytes(request.into_body(), 128 * 1024)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    let mut state = state.lock().unwrap();
    state.puts.push(value);
    if state.conflict {
        return StatusCode::CONFLICT.into_response();
    }
    if state.lose_response_after_write {
        let bytes = STANDARD
            .decode(state.puts.last().unwrap()["content"].as_str().unwrap())
            .unwrap();
        state.current = Some(current(std::str::from_utf8(&bytes).unwrap()));
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    if state.incomplete_receipt {
        return axum::Json(json!({})).into_response();
    }
    axum::Json(json!({"content":{"path":"Formula/kuru.rb","sha":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},"commit":{"sha":"cccccccccccccccccccccccccccccccccccccccc"}})).into_response()
}

fn current(formula: &str) -> Value {
    json!({"type":"file","path":"Formula/kuru.rb","encoding":"base64","content":format!("{}\n",STANDARD.encode(formula)),"size":formula.len(),"sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"})
}

async fn fixture(state: Fixture) -> (Shared, GitHub, tokio::task::JoinHandle<()>) {
    let state = Arc::new(Mutex::new(state));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/", listener.local_addr().unwrap());
    let router = Router::new().fallback(handle).with_state(state.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let api =
        GitHub::with_endpoints(homebrew::TAP_REPOSITORY, "fake-tap-token", &base, &base).unwrap();
    (state, api, task)
}

#[tokio::test]
async fn tap_contents_writes_are_cas_and_identical_retries_have_no_effect() {
    let selected = version("0.12.0");
    let formula = homebrew::generate(selected, homebrew::SOURCE_REPOSITORY, &hashes()).unwrap();
    let old = formula.replace("0.12.0", "0.11.0");
    let (state, api, server) = fixture(Fixture {
        current: Some(current(&old)),
        ..Default::default()
    })
    .await;
    assert!(homebrew::publish(&api, selected, &formula).await.unwrap());
    {
        let mut state = state.lock().unwrap();
        assert_eq!(state.puts.len(), 1);
        assert_eq!(
            state.puts[0]["sha"],
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(state.puts[0]["branch"], "main");
        assert_eq!(
            STANDARD
                .decode(state.puts[0]["content"].as_str().unwrap())
                .unwrap(),
            formula.as_bytes()
        );
        state.current = Some(current(&formula));
    }
    assert!(!homebrew::publish(&api, selected, &formula).await.unwrap());
    assert_eq!(state.lock().unwrap().puts.len(), 1);
    server.abort();
}

#[tokio::test]
async fn lost_write_response_is_reconciled_by_identical_retry_without_another_mutation() {
    let selected = version("0.12.0");
    let formula = homebrew::generate(selected, homebrew::SOURCE_REPOSITORY, &hashes()).unwrap();
    let old = formula.replace("0.12.0", "0.11.0");
    let (state, api, server) = fixture(Fixture {
        current: Some(current(&old)),
        lose_response_after_write: true,
        ..Default::default()
    })
    .await;
    assert!(homebrew::publish(&api, selected, &formula).await.is_err());
    {
        let state = state.lock().unwrap();
        assert_eq!(state.puts.len(), 1);
        assert_eq!(state.current, Some(current(&formula)));
    }
    assert!(!homebrew::publish(&api, selected, &formula).await.unwrap());
    assert_eq!(state.lock().unwrap().puts.len(), 1);
    server.abort();
}

#[tokio::test]
async fn tap_refuses_rollback_same_version_conflicts_and_malformed_state() {
    let formula =
        homebrew::generate(version("0.12.0"), homebrew::SOURCE_REPOSITORY, &hashes()).unwrap();
    let mut bad_encoding = current(&formula);
    bad_encoding["content"] = json!("not base64!");
    let mut bad_size = current(&formula);
    bad_size["size"] = json!(1);
    let mut bad_sha = current(&formula.replace("0.12.0", "0.11.0"));
    bad_sha["sha"] = json!("short");
    let mut bad_kind = current(&formula);
    bad_kind["type"] = json!("symlink");
    let mut bad_path = current(&formula);
    bad_path["path"] = json!("Formula/other.rb");
    let mut absent_version = current("class Kuru < Formula\nend\n");
    absent_version["size"] = json!("class Kuru < Formula\nend\n".len());
    for existing in [
        current(&formula.replace("0.12.0", "0.13.0")),
        current(&format!("{formula}# conflicting same version\n")),
        bad_encoding,
        bad_size,
        bad_sha,
        bad_kind,
        bad_path,
        absent_version,
    ] {
        let (state, api, server) = fixture(Fixture {
            current: Some(existing),
            ..Default::default()
        })
        .await;
        assert!(
            homebrew::publish(&api, version("0.12.0"), &formula)
                .await
                .is_err()
        );
        assert!(state.lock().unwrap().puts.is_empty());
        server.abort();
    }
}

#[tokio::test]
async fn initialized_tap_bootstrap_and_concurrent_update_have_checked_outcomes() {
    let formula =
        homebrew::generate(version("0.12.0"), homebrew::SOURCE_REPOSITORY, &hashes()).unwrap();
    let (state, api, server) = fixture(Fixture::default()).await;
    assert!(
        homebrew::publish(&api, version("0.12.0"), &formula)
            .await
            .unwrap()
    );
    assert!(state.lock().unwrap().puts[0].get("sha").is_none());
    assert!(
        homebrew::publish(&api, version("0.11.0"), &formula)
            .await
            .is_err()
    );
    server.abort();
    for incomplete_receipt in [false, true] {
        let (state, api, server) = fixture(Fixture {
            current: Some(current(&formula.replace("0.12.0", "0.11.0"))),
            conflict: !incomplete_receipt,
            incomplete_receipt,
            ..Default::default()
        })
        .await;
        assert!(
            homebrew::publish(&api, version("0.12.0"), &formula)
                .await
                .is_err()
        );
        assert_eq!(state.lock().unwrap().puts.len(), 1);
        server.abort();
    }
    let wrong = GitHub::new("replygirl/kuru", "fake").unwrap();
    assert!(
        homebrew::publish(&wrong, version("0.12.0"), &formula)
            .await
            .is_err()
    );
}
