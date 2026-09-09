#![forbid(unsafe_code)]

//! A native wire peer for connector integration tests. The owning tests supply
//! ordered I/O steps and validate captured requests with serde_json. This
//! process deliberately uses only std so it can be compiled once by rustc,
//! without recursive Cargo invocations or an external interpreter.

use std::{
    fs::{self, File},
    io::{self, BufRead, Write},
    process, thread,
    time::Duration,
};

fn main() -> io::Result<()> {
    let executable = std::env::current_exe()?;
    let directory = executable.parent().expect("fixture directory");
    let plan = fs::read_to_string(executable.with_extension("plan"))?;
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
            "auth" => {
                let args: Vec<_> = std::env::args().skip(1).collect();
                if !matches!(
                    args.iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        .as_slice(),
                    ["login"] | ["logout"] | ["login", "status"]
                ) {
                    return Err(io::Error::other("unexpected authentication arguments"));
                }
            }
            "exit" => process::exit(argument.parse().expect("exit code")),
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
