//! What can accumulate between runs, counted before and after each run:
//! Kuru and Dolt processes of this user anywhere on the host, the open file
//! (or handle) count and listening TCP ports of processes from the scratch
//! root, and the store staging and interrupted directories under the scratch
//! root. Images are named without their paths and no command line is kept.

use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    time::Instant,
};

use serde::{Deserialize, Serialize};

use super::observe::{Processes, Role, Seen, millis};

/// Bound on one listening-port query (`lsof` on macOS, `NETSTAT.EXE` on
/// Windows); past it the ports are recorded as unavailable.
#[cfg(any(target_os = "macos", windows))]
const QUERY_BOUND: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Census {
    /// Processes running from the scratch root, by role.
    pub ours: BTreeMap<String, usize>,
    /// This user's other Kuru and Dolt processes on the host, by image and
    /// role, for example `dolt sql-server`.
    pub foreign: BTreeMap<String, usize>,
    /// Kuru and Dolt processes elsewhere whose owner could not be read.
    pub owner_unknown: usize,
    /// Open file descriptors (handles on Windows) of the processes from the
    /// scratch root, summed by role.
    pub open_files: BTreeMap<String, usize>,
    /// Processes from the scratch root whose count could not be read.
    pub open_files_unread: usize,
    /// TCP ports the processes from the scratch root listen on, by role;
    /// absent when the query was unavailable.
    pub listening_tcp: Option<BTreeMap<String, Vec<u16>>>,
    /// `*.staging-*` directories in every `data/memory` under the root.
    pub staging_directories: usize,
    /// Entries of every `data/memory/interrupted` under the root.
    pub interrupted_directories: usize,
    /// Time the census took.
    pub cost_ms: f64,
}

impl Census {
    /// Every process from the scratch root.
    pub fn ours_total(&self) -> usize {
        self.ours.values().sum()
    }

    pub fn foreign_total(&self) -> usize {
        self.foreign.values().sum()
    }

    pub fn open_files_total(&self) -> usize {
        self.open_files.values().sum()
    }

    pub fn listening_total(&self) -> Option<usize> {
        self.listening_tcp
            .as_ref()
            .map(|ports| ports.values().map(Vec::len).sum())
    }
}

/// Count everything the census covers under `root`.
pub async fn take(processes: &mut Processes, root: &Path) -> Census {
    let started = Instant::now();
    let seen = processes.census();
    let mut census = of(&seen);
    let ours: Vec<(u32, Role)> = seen
        .iter()
        .filter(|seen| seen.ours)
        .map(|seen| (seen.pid, seen.role))
        .collect();
    census.listening_tcp = listening(&ours).await;
    (census.staging_directories, census.interrupted_directories) = stages(root);
    census.cost_ms = millis(started.elapsed());
    census
}

/// The process part of a census.
pub fn of(seen: &[Seen]) -> Census {
    let mut census = Census::default();
    for process in seen {
        if process.ours {
            *census
                .ours
                .entry(process.role.label().to_owned())
                .or_default() += 1;
            match process.open_files {
                Some(count) => {
                    *census
                        .open_files
                        .entry(process.role.label().to_owned())
                        .or_default() += count;
                }
                None => census.open_files_unread += 1,
            }
            continue;
        }
        match process.mine {
            Some(true) => {
                *census
                    .foreign
                    .entry(format!("{} {}", process.image, process.role.label()))
                    .or_default() += 1;
            }
            Some(false) => {}
            None => census.owner_unknown += 1,
        }
    }
    census
}

/// Store staging and interrupted directories in every iteration's data
/// directory, listed from their parents only.
pub fn stages(root: &Path) -> (usize, usize) {
    let names = |directory: &Path| -> Vec<(String, PathBuf)> {
        std::fs::read_dir(directory)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| {
                        (
                            entry.file_name().to_string_lossy().into_owned(),
                            entry.path(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let (mut staging, mut interrupted) = (0, 0);
    for (name, iteration) in names(root) {
        if !name.starts_with("iteration-") {
            continue;
        }
        let memory = iteration.join("data").join("memory");
        for (name, _) in names(&memory) {
            if name.contains(".staging-") {
                staging += 1;
            }
        }
        interrupted += names(&memory.join("interrupted")).len();
    }
    (staging, interrupted)
}

async fn listening(ours: &[(u32, Role)]) -> Option<BTreeMap<String, Vec<u16>>> {
    let mut by_role: BTreeMap<String, Vec<u16>> = BTreeMap::new();
    if ours.is_empty() {
        return Some(by_role);
    }
    let pids: Vec<u32> = ours.iter().map(|(pid, _)| *pid).collect();
    let ports = ports(&pids).await?;
    for (pid, role) in ours {
        if let Some(found) = ports.get(pid) {
            by_role
                .entry(role.label().to_owned())
                .or_default()
                .extend(found);
        }
    }
    for ports in by_role.values_mut() {
        ports.sort_unstable();
        ports.dedup();
    }
    Some(by_role)
}

#[cfg(target_os = "linux")]
async fn ports(pids: &[u32]) -> Option<HashMap<u32, Vec<u16>>> {
    let mut inodes = HashMap::new();
    for table in ["/proc/net/tcp", "/proc/net/tcp6"] {
        if let Ok(text) = std::fs::read_to_string(table) {
            inodes.extend(parse_proc_net_tcp(&text));
        }
    }
    let mut found: HashMap<u32, Vec<u16>> = HashMap::new();
    for pid in pids {
        let links: Vec<PathBuf> = match std::fs::read_dir(format!("/proc/{pid}/fd")) {
            Ok(entries) => entries.flatten().map(|entry| entry.path()).collect(),
            Err(_) => continue,
        };
        for link in links {
            let inode = std::fs::read_link(&link)
                .ok()
                .and_then(|target| socket_inode(&target.to_string_lossy()));
            if let Some(port) = inode.and_then(|inode| inodes.get(&inode)) {
                found.entry(*pid).or_default().push(*port);
            }
        }
    }
    Some(found)
}

#[cfg(target_os = "macos")]
async fn ports(pids: &[u32]) -> Option<HashMap<u32, Vec<u16>>> {
    let list = pids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    // lsof exits 1 when a listed process has exited or nothing matched; its
    // output still lists every match.
    let output = query(
        "/usr/sbin/lsof".as_ref(),
        &["-nP", "-a", "-p", &list, "-iTCP", "-sTCP:LISTEN", "-Fpn"],
    )
    .await?;
    Some(parse_lsof(&String::from_utf8_lossy(&output.stdout)))
}

#[cfg(windows)]
async fn ports(pids: &[u32]) -> Option<HashMap<u32, Vec<u16>>> {
    let system = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    let netstat = PathBuf::from(system).join("System32").join("NETSTAT.EXE");
    let output = query(netstat.as_os_str(), &["-ano"]).await?;
    if !output.status.success() {
        return None;
    }
    let mut found = parse_netstat(&String::from_utf8_lossy(&output.stdout));
    found.retain(|pid, _| pids.contains(pid));
    Some(found)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
async fn ports(_: &[u32]) -> Option<HashMap<u32, Vec<u16>>> {
    None
}

/// Run a stock OS query through the delivery tool's process boundary.
#[cfg(any(target_os = "macos", windows))]
async fn query(program: &std::ffi::OsStr, args: &[&str]) -> Option<std::process::Output> {
    let mut command = crate::command::Command::new(program);
    command.args(args).kill_on_drop(true);
    tokio::time::timeout(QUERY_BOUND, command.output())
        .await
        .ok()?
        .ok()
}

/// `(socket inode, local port)` of every listening socket in a
/// `/proc/net/tcp` or `tcp6` table.
pub fn parse_proc_net_tcp(text: &str) -> Vec<(u64, u16)> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.get(3) != Some(&"0A") {
                return None;
            }
            let port = u16::from_str_radix(fields.get(1)?.rsplit(':').next()?, 16).ok()?;
            let inode = fields.get(9)?.parse().ok()?;
            Some((inode, port))
        })
        .collect()
}

/// The inode of a `/proc/<pid>/fd` link to `socket:[<inode>]`.
pub fn socket_inode(target: &str) -> Option<u64> {
    target
        .strip_prefix("socket:[")?
        .strip_suffix(']')?
        .parse()
        .ok()
}

/// Listening ports by process from `lsof -F pn` output.
pub fn parse_lsof(text: &str) -> HashMap<u32, Vec<u16>> {
    let mut found: HashMap<u32, Vec<u16>> = HashMap::new();
    let mut pid = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix('p') {
            pid = value.parse().ok();
        } else if let (Some(pid), Some(name)) = (pid, line.strip_prefix('n'))
            && let Some(port) = name.rsplit(':').next().and_then(|port| port.parse().ok())
        {
            found.entry(pid).or_default().push(port);
        }
    }
    found
}

/// Listening TCP ports by process from `netstat -ano` output. A listening
/// row is recognised by its unconnected remote address, which does not
/// depend on the display language of the state column.
pub fn parse_netstat(text: &str) -> HashMap<u32, Vec<u16>> {
    let mut found: HashMap<u32, Vec<u16>> = HashMap::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 5
            || fields[0] != "TCP"
            || !(fields[2] == "0.0.0.0:0" || fields[2] == "[::]:0")
        {
            continue;
        }
        let port = fields[1]
            .rsplit(':')
            .next()
            .and_then(|port| port.parse().ok());
        let pid = fields.last().and_then(|pid| pid.parse().ok());
        if let (Some(port), Some(pid)) = (port, pid) {
            found.entry(pid).or_default().push(port);
        }
    }
    found
}
