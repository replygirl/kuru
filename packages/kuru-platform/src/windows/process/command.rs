//! Explicit command resolution and cmd transport; creation stays in NativeSpawnSpec.
//! Batch escaping is adapted from Rust 1.98.1's std/sys/args/windows.rs,
//! https://github.com/rust-lang/rust/blob/1.98.1/library/std/src/sys/args/windows.rs
//! under the MIT license in RUST-LICENSE-MIT. No raw-tail fallback applies to argv.

use super::{CommandSyntax, NativeSpawnSpec, compare_keys, invalid, wide};
use std::{
    ffi::{OsStr, OsString},
    io,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Component, Path, PathBuf, Prefix},
};
use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;

/// Resolve the OS directory without trusting PATH, COMSPEC or user variables.
pub fn system_directory() -> io::Result<PathBuf> {
    let mut buffer = vec![0u16; 260];
    loop {
        // SAFETY: buffer owns length writable UTF-16 units; API writes at most
        // the supplied size and reports a larger required size for retry.
        let length =
            unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
        if length == 0 {
            return Err(io::Error::last_os_error());
        }
        if length < buffer.len() {
            return Ok(PathBuf::from(OsString::from_wide(&buffer[..length])));
        }
        if length > 32767 {
            return Err(invalid("system directory exceeds Windows path limit"));
        }
        buffer.resize(length + 1, 0);
    }
}

/// Windows' ordinal key comparison, without lossy Unicode conversion.
pub fn environment_key_eq(left: &OsStr, right: &OsStr) -> bool {
    let left: Vec<_> = left.encode_wide().chain([0]).collect();
    let right: Vec<_> = right.encode_wide().chain([0]).collect();
    compare_keys(&left, &right).is_eq()
}

/// Capture at the caller and merge without changing the parent environment.
/// Duplicate case-equivalent keys within either input are rejected, while an
/// explicit override replaces its matching inherited key.
pub fn merge_environment(
    base: impl IntoIterator<Item = (OsString, OsString)>,
    overrides: impl IntoIterator<Item = (OsString, OsString)>,
) -> io::Result<Vec<(OsString, OsString)>> {
    let mut base: Vec<_> = base.into_iter().collect();
    let overrides: Vec<_> = overrides.into_iter().collect();
    super::environment(base.clone())?;
    super::environment(overrides.clone())?;
    for (key, value) in overrides {
        base.retain(|(old, _)| !environment_key_eq(old, &key));
        base.push((key, value));
    }
    Ok(base)
}

fn value<'a>(environment: &'a [(OsString, OsString)], key: &str) -> Option<&'a OsStr> {
    environment
        .iter()
        .find(|(name, _)| environment_key_eq(name, OsStr::new(key)))
        .map(|(_, value)| value.as_os_str())
}

fn extensions(environment: &[(OsString, OsString)]) -> Vec<OsString> {
    let configured = value(environment, "PATHEXT").unwrap_or(OsStr::new(".COM;.EXE;.BAT;.CMD"));
    let mut result = Vec::new();
    for item in configured.to_string_lossy().split(';') {
        if [".exe", ".com", ".cmd", ".bat"]
            .iter()
            .any(|known| item.eq_ignore_ascii_case(known))
            && !result
                .iter()
                .any(|old: &OsString| old.eq_ignore_ascii_case(item))
        {
            result.push(item.into());
        }
    }
    result
}

/// No current-directory or parent-PATH fallback. Explicit relative paths are
/// anchored at cwd; unusable search entries never hide later usable entries.
pub fn resolve_executable(
    program: &OsStr,
    cwd: &Path,
    environment: &[(OsString, OsString)],
) -> io::Result<PathBuf> {
    wide(program)?;
    if program.is_empty() || !cwd.is_absolute() {
        return Err(invalid(
            "command and absolute working directory are required",
        ));
    }
    let path = Path::new(program);
    if path
        .components()
        .any(|part| matches!(part, Component::Prefix(_) | Component::RootDir))
        && !path.is_absolute()
    {
        return Err(invalid(
            "drive-relative and rooted-without-drive commands are ambiguous",
        ));
    }
    let bare = path.components().count() == 1
        && matches!(path.components().next(), Some(Component::Normal(_)));
    let candidates = |base: PathBuf| {
        let mut values = vec![base.clone()];
        if base.extension().is_none() {
            values.extend(extensions(environment).into_iter().map(|suffix| {
                let mut name = base.as_os_str().to_os_string();
                name.push(suffix);
                PathBuf::from(name)
            }));
        }
        values
    };
    let paths: Vec<PathBuf> = if bare {
        value(environment, "PATH")
            .map(std::env::split_paths)
            .into_iter()
            .flatten()
            .filter(|entry| entry.is_absolute())
            .flat_map(|entry| candidates(entry.join(path)))
            .collect()
    } else {
        candidates(if path.is_absolute() {
            path.into()
        } else {
            cwd.join(path)
        })
    };
    for candidate in paths {
        if candidate.is_file() {
            return candidate.canonicalize();
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "configured executable was not found in its explicit path or child PATH",
    ))
}

fn is_cmd(program: &OsStr, cwd: &Path, environment: &[(OsString, OsString)]) -> io::Result<bool> {
    if program.eq_ignore_ascii_case("cmd") || program.eq_ignore_ascii_case("cmd.exe") {
        return Ok(true);
    }
    if !Path::new(program)
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("cmd.exe"))
    {
        return Ok(false);
    }
    Ok(resolve_executable(program, cwd, environment)?
        == system_directory()?.join("cmd.exe").canonicalize()?)
}

/// Intent supplied by an explicit configured program. Native executables keep
/// argv; batch files use literal batch encoding; configured cmd accepts either
/// separate target/argv or one deliberately authored source tail after /c.
pub fn configured_command(
    program: &OsStr,
    args: &[OsString],
    cwd: &Path,
    environment: Vec<(OsString, OsString)>,
) -> io::Result<NativeSpawnSpec> {
    let cwd = cwd.canonicalize()?;
    let mut spec = if is_cmd(program, &cwd, &environment)? {
        let separator = args
            .iter()
            .position(|arg| arg.eq_ignore_ascii_case("/c"))
            .ok_or_else(|| {
                invalid("configured cmd requires /c followed by a target or one source argument")
            })?;
        let switches = args[..separator].to_vec();
        let tail = &args[separator + 1..];
        match tail {
            [] => return Err(invalid("configured cmd /c lacks its command")),
            [source] => {
                validate_switches(&switches, false)?;
                let mut spec =
                    NativeSpawnSpec::new(system_directory()?.join("cmd.exe"), cwd.clone());
                spec.syntax = CommandSyntax::CmdSource {
                    switches,
                    source: source.clone(),
                };
                spec
            }
            [target, rest @ ..] => {
                validate_switches(&switches, true)?;
                let target = resolve_executable(target, &cwd, &environment)?;
                let mut spec = NativeSpawnSpec::new(target, cwd.clone());
                spec.args = rest.to_vec();
                spec.syntax = CommandSyntax::CmdInvocation { switches };
                spec
            }
        }
    } else {
        let program = resolve_executable(program, &cwd, &environment)?;
        let mut spec = NativeSpawnSpec::new(program, cwd.clone());
        spec.args = args.to_vec();
        if spec
            .executable
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat"))
        {
            spec.syntax = CommandSyntax::CmdInvocation {
                switches: Vec::new(),
            };
        }
        spec
    };
    spec.environment = environment;
    Ok(spec)
}

fn validate_switches(switches: &[OsString], literal: bool) -> io::Result<()> {
    for switch in switches {
        let common = ["/d", "/s", "/q", "/e:on", "/v:off", "/a", "/u"];
        if !common
            .iter()
            .any(|known| switch.eq_ignore_ascii_case(known))
            && (literal
                || !["/e:off", "/v:on"]
                    .iter()
                    .any(|known| switch.eq_ignore_ascii_case(known)))
        {
            return Err(invalid(
                "unsupported cmd switch; expansion options require one explicit source argument",
            ));
        }
    }
    Ok(())
}

fn prefix(switches: &[OsString], literal: bool) -> io::Result<Vec<u16>> {
    validate_switches(switches, literal)?;
    let mut result: Vec<_> = if literal {
        "cmd.exe /e:ON /v:OFF /d"
    } else {
        "cmd.exe"
    }
    .encode_utf16()
    .collect();
    for switch in switches {
        result.push(b' ' as u16);
        let encoded = wide(switch)?;
        result.extend_from_slice(&encoded[..encoded.len() - 1]);
    }
    result.extend(" /c ".encode_utf16());
    Ok(result)
}

fn bounded(mut text: Vec<u16>) -> io::Result<Vec<u16>> {
    if text.iter().any(|unit| [0, 10, 13].contains(unit)) {
        return Err(invalid("cmd transport cannot represent NUL or line breaks"));
    }
    text.push(0);
    if text.len() > 8191 {
        return Err(invalid("cmd command line exceeds 8191 UTF-16 units"));
    }
    Ok(text)
}

pub(super) fn source_line(switches: &[OsString], source: &OsStr) -> io::Result<Vec<u16>> {
    let mut result = prefix(switches, false)?;
    result.extend(source.encode_wide());
    bounded(result)
}

fn batch_path(path: &Path) -> io::Result<OsString> {
    let original: Vec<_> = path.as_os_str().encode_wide().collect();
    if original.contains(&(b'%' as u16)) || original.contains(&(b'"' as u16)) {
        return Err(invalid(
            "batch target path contains unsupported quote or percent expansion; use an explicit source expression",
        ));
    }
    let mut candidate = path.to_path_buf();
    if let Some(Component::Prefix(prefix)) = path.components().next() {
        candidate = match prefix.kind() {
            Prefix::VerbatimDisk(drive) if original.len() < 260 => {
                let mut normal = OsString::from(format!("{}:\\", char::from(drive)));
                for part in path.components().skip(2) {
                    normal.push(part.as_os_str());
                    normal.push("\\");
                }
                let mut units: Vec<_> = normal.encode_wide().collect();
                if units.last() == Some(&(b'\\' as u16)) {
                    units.pop();
                }
                PathBuf::from(OsString::from_wide(&units))
            }
            Prefix::VerbatimUNC(_, _) | Prefix::Verbatim(_) | Prefix::VerbatimDisk(_) => {
                return Err(invalid(
                    "batch target cannot use this verbatim Windows path",
                ));
            }
            _ => candidate,
        };
    }
    if candidate.canonicalize()? != path.canonicalize()? {
        return Err(invalid("batch target path conversion changes its identity"));
    }
    Ok(candidate.into_os_string())
}

pub(super) fn invocation_line(
    target: &Path,
    args: &[OsString],
    switches: &[OsString],
    cwd: &Path,
) -> io::Result<Vec<u16>> {
    if matches!(cwd.components().next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::UNC(_, _) | Prefix::VerbatimUNC(_, _)))
    {
        return Err(invalid(
            "cmd does not preserve a UNC working directory; use a native executable",
        ));
    }
    let script = batch_path(target)?;
    let mut result = prefix(switches, true)?;
    result.push(b'"' as u16);
    result.push(b'"' as u16);
    result.extend(script.encode_wide());
    result.push(b'"' as u16);
    for arg in args {
        result.push(b' ' as u16);
        append_batch_arg(&mut result, arg)?;
    }
    result.push(b'"' as u16);
    bounded(result)
}

// Adapted from Rust 1.98.1 append_bat_arg (see module license/source).
fn append_batch_arg(output: &mut Vec<u16>, arg: &OsStr) -> io::Result<()> {
    let units: Vec<_> = arg.encode_wide().collect();
    if units.iter().any(|unit| [0, 10, 13].contains(unit)) {
        return Err(invalid("batch arguments cannot contain NUL or line breaks"));
    }
    let quote = units.is_empty()
        || units.last() == Some(&(b'\\' as u16))
        || arg.to_string_lossy().chars().any(|ch| {
            (ch.is_ascii() && !(ch.is_ascii_alphanumeric() || r"#$*+-./:?@\_".contains(ch)))
                || ch.is_control()
        });
    if quote {
        output.push(b'"' as u16);
    }
    let mut slashes = 0;
    for unit in units {
        if unit == b'\\' as u16 {
            slashes += 1;
        } else {
            if unit == b'"' as u16 {
                output.extend(std::iter::repeat_n(b'\\' as u16, slashes));
                output.push(b'"' as u16);
            } else if unit == b'%' as u16 {
                output.extend("%%cd:~,".encode_utf16());
            }
            slashes = 0;
        }
        output.push(unit);
    }
    if quote {
        output.extend(std::iter::repeat_n(b'\\' as u16, slashes));
        output.push(b'"' as u16);
    }
    Ok(())
}
