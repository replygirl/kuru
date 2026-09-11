#![forbid(unsafe_code)]

//! A native wire peer for connector integration tests. The owning tests supply
//! ordered I/O steps and validate captured requests with serde_json. This
//! process deliberately uses only std so it can be compiled once by rustc,
//! without recursive Cargo invocations or an external interpreter.

use std::{
    fs::{self, File},
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    process, thread,
    time::Duration,
};

fn main() -> io::Result<()> {
    // The harness supplies an absolute argv[0] for the fixture's own plan.
    // Executable discovery describes shared code, not this invocation's data.
    let executable = PathBuf::from(std::env::args_os().next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "missing fixture invocation path",
        )
    })?);
    if !executable.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "fixture invocation path must be absolute",
        ));
    }
    let result = run(&executable);
    if let Err(error) = &result {
        // RPC intentionally suppresses child stderr. Keep unexpected fixture
        // failures beside its transcript so the owning test can report them.
        let _ = fs::write(
            executable
                .parent()
                .unwrap()
                .join(format!("{}.error", process::id())),
            error.to_string(),
        );
    }
    result
}

fn run(executable: &Path) -> io::Result<()> {
    let directory = executable.parent().expect("fixture directory");
    fs::write(
        directory.join(format!("{}.started", process::id())),
        format!(
            "invocation={executable:?}\ncurrent_exe={:?}\n",
            std::env::current_exe()
        ),
    )?;
    let plan_path = executable.with_extension("plan");
    let plan = fs::read_to_string(&plan_path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("read fixture plan {}: {error}", plan_path.display()),
        )
    })?;
    let mut transcript = File::create(directory.join(format!("{}.requests", process::id())))?;
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    for step in plan.lines() {
        let (operation, argument) = step.split_once(' ').unwrap_or((step, ""));
        match operation {
            "read" | "eof" => {
                let mut line = String::new();
                let count = input.read_line(&mut line)?;
                if count > 0 {
                    transcript.write_all(line.as_bytes())?;
                    transcript.flush()?;
                }
                if (count == 0) != (operation == "eof") {
                    return Err(io::Error::other(format!(
                        "unexpected input during {operation}"
                    )));
                }
            }
            "write" => {
                writeln!(output, "{argument}")?;
                output.flush()?;
            }
            "repeat" => {
                let count = argument.parse::<usize>().expect("repeat count");
                output.write_all(&vec![b'x'; count])?;
                output.write_all(b"\n")?;
                output.flush()?;
            }
            "sleep" => {
                thread::sleep(Duration::from_millis(
                    argument.parse().expect("milliseconds"),
                ));
            }
            other => {
                return Err(io::Error::other(format!(
                    "unknown fixture operation {other}"
                )));
            }
        }
    }
    fs::write(
        directory.join(format!("{}.done", process::id())),
        b"complete",
    )
}
