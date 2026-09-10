//! Reproduce hook inheritance entirely inside disposable child processes.
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

pub const CHILD: &str = "KURU_TEST_FOREIGN_REPOSITORY_CHILD";

pub struct ForeignRepository {
    temp: tempfile::TempDir,
    before: BTreeMap<PathBuf, Vec<u8>>,
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, path: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root).unwrap().into(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

impl ForeignRepository {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("caller");
        fs::create_dir(&root).unwrap();
        fs::write(
            temp.path().join("global-config"),
            "[user]\n signingkey = fixture-signing-key\n",
        )
        .unwrap();
        fs::write(
            root.join("caller.txt"),
            "caller work must remain untouched\n",
        )
        .unwrap();
        // Fixture setup must be safe even before production sanitization exists.
        // It inherits no caller Git configuration, hooks or repository selectors.
        let trace = temp.path().join("setup-trace.jsonl");
        for args in [
            &["init", "-b", "main"][..],
            &["config", "user.name", "Foreign caller"],
            &["config", "user.email", "caller@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["add", "caller.txt"],
            &["commit", "-m", "fix: sentinel caller commit"],
        ] {
            let output = Command::new("git")
                // The sentinel must be quiescent before its complete snapshot.
                .args(["-c", "maintenance.auto=false", "-c", "gc.auto=0"])
                .args(args)
                .current_dir(&root)
                .env_clear()
                .env("PATH", std::env::var_os("PATH").unwrap())
                .env("HOME", temp.path())
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_TRACE2_EVENT", &trace)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "isolated fixture Git failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let events: Vec<serde_json::Value> = fs::read_to_string(trace)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert!(events.iter().any(|event| event["event"] == "start"));
        for event in events
            .iter()
            .filter(|event| event["event"] == "child_start")
        {
            let args = event["argv"].as_array().unwrap();
            assert!(
                !args.iter().any(|arg| arg == "maintenance" || arg == "gc"),
                "sentinel setup launched background-capable Git housekeeping: {args:?}"
            );
        }
        for name in ["grafts", "shallow"] {
            fs::write(root.join(".git").join(name), "").unwrap();
        }
        let before = snapshot(&root);
        Self { temp, before }
    }

    pub async fn assert_test_isolated(&self, test: &str) {
        let root = self.temp.path().join("caller");
        let git = root.join(".git");
        let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", test, "--nocapture"])
            .current_dir(&root)
            .env(CHILD, "1")
            .env("HOME", self.temp.path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.temp.path().join("global-config"))
            .env("GIT_DIR", &git)
            .env("GIT_COMMON_DIR", &git)
            .env("GIT_WORK_TREE", &root)
            .env("GIT_IMPLICIT_WORK_TREE", "0")
            .env("GIT_INDEX_FILE", git.join("index"))
            .env("GIT_OBJECT_DIRECTORY", git.join("objects"))
            .env("GIT_ALTERNATE_OBJECT_DIRECTORIES", git.join("objects"))
            .env("GIT_CONFIG", git.join("config"))
            .env("GIT_CONFIG_PARAMETERS", "'fixture.hook=poisoned'")
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "fixture.hook")
            .env("GIT_CONFIG_VALUE_0", "poisoned")
            .env("GIT_GRAFT_FILE", git.join("grafts"))
            .env("GIT_SHALLOW_FILE", git.join("shallow"))
            .env("GIT_NO_REPLACE_OBJECTS", "1")
            .env("GIT_REPLACE_REF_BASE", "refs/foreign-replacements/")
            .env("GIT_PREFIX", "foreign-prefix/")
            .env("GIT_SSH_COMMAND", "fixture-ssh")
            .env("SSH_AUTH_SOCK", "fixture-agent")
            .env("GIT_ASKPASS", "fixture-askpass")
            .env("OPENAI_API_KEY", "fixture-inherited-key")
            .env_remove("GITHUB_TOKEN")
            .env_remove("GH_TOKEN")
            .kill_on_drop(true);
        let output = tokio::time::timeout(Duration::from_secs(60), command.output())
            .await
            .expect("foreign-repository child timed out")
            .unwrap();
        let after = snapshot(&root);
        let changed: Vec<_> = self
            .before
            .keys()
            .chain(after.keys())
            .filter(|path| self.before.get(*path) != after.get(*path))
            .collect();
        assert!(
            changed.is_empty(),
            "{test} changed caller repository files: {changed:?}"
        );
        assert!(
            output.status.success(),
            "{test} failed under the foreign hook environment:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "child filter did not run the requested test"
        );
    }
}
