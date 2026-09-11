#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    use std::{
        fs::{self, OpenOptions},
        io::Write,
        path::PathBuf,
    };
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if matches!(arguments.as_slice(), [directory, _, action, task]
        if directory == "-C" && action == "run" && task == "//apps/kuru-tui:install")
    {
        // Observe the public PowerShell entrypoint before any real mise task
        // activation. This fixture does not stand in for a native source build.
        let mut log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(std::env::var_os("KURU_CLI_FIXTURE_SETUP_LOG").expect("isolated setup log"))?;
        serde_json::to_writer(
            &mut log,
            &serde_json::json!({
                "arguments": arguments,
                "no_hooks": std::env::var("MISE_NO_HOOKS").ok(),
                "auto_install": std::env::var("MISE_TASK_RUN_AUTO_INSTALL").ok(),
                "install_dir": std::env::var("KURU_INSTALL_DIR").ok(),
            }),
        )?;
        writeln!(log)?;
        log.flush()?;
        let status = std::env::var("KURU_CLI_FIXTURE_SETUP_EXIT")?.parse::<i32>()?;
        std::process::exit(status);
    }
    if arguments.first().is_some_and(|argument| argument == "-C") {
        for selector in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_CONFIG_COUNT",
            "CARGO_BUILD_TARGET",
        ] {
            anyhow::ensure!(
                std::env::var_os(selector).is_none(),
                "source build retained foreign selection: {selector}"
            );
        }
        anyhow::ensure!(
            std::env::var("GIT_SSH_COMMAND").as_deref() == Ok("fixture-ssh-command")
                && std::env::var("GIT_CONFIG_GLOBAL").as_deref() == Ok("fixture-global-config"),
            "source build discarded unrelated Git authentication/configuration controls"
        );
    }
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(std::env::var_os("KURU_CLI_FIXTURE_LOG").expect("isolated log"))?;
    serde_json::to_writer(&mut log, &arguments)?;
    writeln!(log)?;
    match arguments.as_slice() {
        [action, task] if action == "run" && task == "//packages/kuru-memory:prefetch" => {
            // The caller has already provisioned and verified the real bundled
            // engine. Only the mise routing is substituted, never PE inspection.
            let engine = PathBuf::from(
                std::env::var_os("KURU_CLI_FIXTURE_ENGINE").expect("verified managed engine"),
            );
            anyhow::ensure!(
                engine.is_file() && engine.canonicalize()? == engine,
                "prefetch fixture requires the canonical managed executable"
            );
            log.flush()?;
            println!("{}", engine.display());
        }
        [action] if action == "login" || action == "logout" => {}
        [action, option]
            if action == "login" && (option == "status" || option == "--device-auth") => {}
        [directory, _, action, tool]
            if directory == "-C" && action == "install" && tool == "rust" => {}
        [directory, _, action, task, separator, target, triple]
            if directory == "-C"
                && action == "run"
                && task == "//apps/kuru-tui:build:release"
                && separator == "--"
                && target == "--target"
                && triple == "x86_64-pc-windows-msvc" =>
        {
            let destination = PathBuf::from(
                std::env::var_os("KURU_CLI_FIXTURE_TARGET").expect("isolated target"),
            )
            .join(triple)
            .join("release");
            fs::create_dir_all(&destination)?;
            fs::copy(
                std::env::var_os("KURU_CLI_FIXTURE_BINARY").expect("trusted source fixture"),
                destination.join("kuru.exe"),
            )?;
        }
        _ => anyhow::bail!("unexpected compiled CLI fixture arguments: {arguments:?}"),
    }
    Ok(())
}
