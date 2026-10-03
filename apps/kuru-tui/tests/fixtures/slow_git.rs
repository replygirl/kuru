//! A `git` stand-in for the project-local index check: it answers
//! `ls-files --error-unmatch` with the untracked status (exit 1), but only
//! after pacing itself past the former 5 s cap, as a large index or slow disk
//! would. Copied into a temporary PATH directory as `git` or `git.exe`, so the
//! spawned `kuru` resolves it like the real executable on every platform.

fn main() {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if !arguments.iter().any(|argument| argument == "ls-files") {
        // Any other Git use would mean the check changed shape; report it as
        // an ambiguous status rather than a plausible answer.
        eprintln!("slow git fixture received unexpected arguments: {arguments:?}");
        std::process::exit(2);
    }
    std::thread::sleep(std::time::Duration::from_millis(5_500));
    std::process::exit(1);
}
