//! Strict single-disk ZIP codecs for explicitly named payloads.
//!
//! Parse physical records before decoding: name-deduplicating ZIP APIs cannot
//! prove that an archive contains only one physical copy of each member.

use anyhow::{Context, Result, ensure};
use flate2::{Decompress, FlushDecompress, Status};
use std::collections::{HashMap, HashSet};
use std::io::{self, Cursor, Seek, SeekFrom, Write};
use std::ops::Range;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemberKind {
    File,
    Directory,
}

#[derive(Clone, Copy, Debug)]
pub struct MemberSpec<'a> {
    pub name: &'a str,
    pub kind: MemberKind,
    pub max_bytes: u64,
    pub exact_bytes: Option<u64>,
    /// Full Unix type and permission bits, when the caller pins that metadata.
    pub unix_mode: Option<u32>,
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_compressed_bytes: u64,
    pub max_expanded_bytes: u64,
    /// Accept the audited, central-only NTFS three-timestamp field.
    pub allow_ntfs_timestamps: bool,
}

pub struct WriteMember<'a> {
    pub name: &'a str,
    pub kind: MemberKind,
    pub bytes: &'a [u8],
    pub executable: bool,
}

#[derive(Debug)]
struct Record {
    name: String,
    data: Range<usize>,
    expanded: u64,
    crc: u32,
    method: u16,
}

#[derive(Debug)]
pub struct Archive<'a> {
    bytes: &'a [u8],
    records: Vec<Record>,
}

fn slice(bytes: &[u8], start: usize, length: usize) -> Result<&[u8]> {
    let end = start.checked_add(length).context("ZIP offset overflow")?;
    bytes.get(start..end).context("truncated ZIP record")
}

fn u16_at(bytes: &[u8], offset: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(slice(bytes, offset, 2)?.try_into()?))
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(slice(bytes, offset, 4)?.try_into()?))
}

fn checked_name(name: &str, kind: MemberKind) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name.len() <= 1024
            && name.is_ascii()
            && !name.bytes().any(|byte| byte.is_ascii_control())
            && !name.contains(['\\', ':'])
            && name.ends_with('/') == (kind == MemberKind::Directory),
        "invalid fixed ZIP member name"
    );
    let path = name.strip_suffix('/').unwrap_or(name);
    ensure!(
        path.split('/')
            .all(|part| !part.is_empty() && part != "." && part != ".."),
        "ambiguous ZIP member path"
    );
    Ok(())
}

fn extra(bytes: &[u8], allowed: bool) -> Result<()> {
    if bytes.is_empty() {
        return Ok(());
    }
    ensure!(
        allowed
            && bytes.len() == 36
            && u16_at(bytes, 0)? == 0x000a
            && u16_at(bytes, 2)? == 32
            && u32_at(bytes, 4)? == 0
            && u16_at(bytes, 8)? == 1
            && u16_at(bytes, 10)? == 24,
        "unsupported or malformed ZIP extra field"
    );
    Ok(())
}

impl<'a> Archive<'a> {
    pub fn open(bytes: &'a [u8], expected: &[MemberSpec<'_>], limits: Limits) -> Result<Self> {
        ensure!(
            bytes.len() as u64 <= limits.max_compressed_bytes,
            "compressed ZIP exceeds limit"
        );
        ensure!(
            !expected.is_empty() && expected.len() <= 256,
            "invalid ZIP inventory size"
        );
        let mut policy = HashMap::new();
        for member in expected {
            checked_name(member.name, member.kind)?;
            ensure!(
                policy.insert(member.name, member).is_none(),
                "duplicate expected ZIP member"
            );
        }
        let end = bytes
            .len()
            .checked_sub(22)
            .context("missing ZIP end record")?;
        let footer = slice(bytes, end, 22)?;
        ensure!(
            u32_at(footer, 0)? == 0x0605_4b50,
            "ZIP end record or trailing data is invalid"
        );
        ensure!(
            u16_at(footer, 4)? == 0 && u16_at(footer, 6)? == 0 && u16_at(footer, 20)? == 0,
            "multi-disk ZIP or archive comment is unsupported"
        );
        let count = u16_at(footer, 10)? as usize;
        ensure!(
            count == expected.len() && u16_at(footer, 8)? as usize == count,
            "ZIP inventory count mismatch"
        );
        let central_start = u32_at(footer, 16)? as usize;
        let central_size = u32_at(footer, 12)? as usize;
        ensure!(
            central_start.checked_add(central_size) == Some(end),
            "inconsistent ZIP central directory bounds"
        );
        let mut cursor = central_start;
        let mut physical = Vec::with_capacity(count);
        let mut names = HashSet::new();
        let mut offsets = HashSet::new();
        let mut expanded_total = 0u64;
        for _ in 0..count {
            let header = slice(bytes, cursor, 46)?;
            ensure!(
                u32_at(header, 0)? == 0x0201_4b50,
                "invalid ZIP central header"
            );
            ensure!(
                header[5] == 3 && matches!(u16_at(header, 6)?, 10 | 20),
                "unsupported ZIP creator or required version"
            );
            ensure!(
                u16_at(header, 8)? == 0,
                "ZIP encryption, descriptors and unsupported flags are forbidden"
            );
            let method = u16_at(header, 10)?;
            ensure!(
                matches!(method, 0 | 8),
                "unsupported ZIP compression method"
            );
            let crc = u32_at(header, 16)?;
            let compressed = u32_at(header, 20)?;
            let expanded = u32_at(header, 24)?;
            ensure!(
                compressed != u32::MAX && expanded != u32::MAX,
                "ZIP64 sizes are unsupported"
            );
            let name_length = u16_at(header, 28)? as usize;
            let extra_length = u16_at(header, 30)? as usize;
            ensure!(
                u16_at(header, 32)? == 0 && u16_at(header, 34)? == 0 && u16_at(header, 36)? == 0,
                "ZIP comments, split members and unsupported attributes are forbidden"
            );
            let name_bytes = slice(bytes, cursor + 46, name_length)?;
            let name = std::str::from_utf8(name_bytes).context("ZIP member name is not UTF-8")?;
            let member = policy.get(name).context("unexpected ZIP member")?;
            ensure!(
                names.insert(name.to_owned()),
                "duplicate physical ZIP member"
            );
            let attributes = u32_at(header, 38)?;
            let mode = attributes >> 16;
            let kind = match mode & 0o170000 {
                0o100000 => MemberKind::File,
                0o040000 => MemberKind::Directory,
                _ => anyhow::bail!("ZIP links and special files are forbidden"),
            };
            checked_name(name, kind)?;
            ensure!(
                kind == member.kind
                    && mode & 0o7000 == 0
                    // p7zip's 0x8000 POSIX-mode marker accompanies the audited
                    // upstream attributes (7zip ZipItem.cpp/GetWinAttrib).
                    // Low bits are never applied to output files; pinned Unix
                    // type/mode governs the accepted payload.
                    && attributes & 0xffff & !0x8030 == 0
                    && (attributes & 0x10 == 0 || kind == MemberKind::Directory)
                    && member.unix_mode.is_none_or(|expected| expected == mode),
                "ZIP member type or permissions mismatch"
            );
            ensure!(
                expanded as u64 <= member.max_bytes
                    && member
                        .exact_bytes
                        .is_none_or(|size| size == expanded as u64),
                "ZIP member expanded size violates policy"
            );
            expanded_total = expanded_total
                .checked_add(expanded as u64)
                .context("ZIP expanded size overflow")?;
            ensure!(
                expanded_total <= limits.max_expanded_bytes,
                "expanded ZIP exceeds limit"
            );
            if kind == MemberKind::Directory {
                ensure!(
                    method == 0 && compressed == 0 && expanded == 0 && crc == 0,
                    "ZIP directory has payload"
                );
            }
            if method == 0 {
                ensure!(compressed == expanded, "stored ZIP sizes disagree");
            }
            extra(
                slice(bytes, cursor + 46 + name_length, extra_length)?,
                limits.allow_ntfs_timestamps,
            )?;
            cursor = cursor
                .checked_add(46 + name_length + extra_length)
                .context("ZIP central offset overflow")?;
            ensure!(cursor <= end, "ZIP central member exceeds directory");

            let local_offset = u32_at(header, 42)? as usize;
            ensure!(offsets.insert(local_offset), "duplicate ZIP local offset");
            let local = slice(bytes, local_offset, 30)?;
            ensure!(u32_at(local, 0)? == 0x0403_4b50, "invalid ZIP local header");
            ensure!(
                slice(local, 4, 22)? == slice(header, 6, 22)?
                    && u16_at(local, 26)? as usize == name_length
                    && u16_at(local, 28)? == 0,
                "ZIP local and central metadata disagree"
            );
            ensure!(
                slice(bytes, local_offset + 30, name_length)? == name_bytes,
                "ZIP local and central names disagree"
            );
            let data_start = local_offset
                .checked_add(30 + name_length)
                .context("ZIP data offset overflow")?;
            let data_end = data_start
                .checked_add(compressed as usize)
                .context("ZIP data size overflow")?;
            ensure!(
                data_end <= central_start,
                "ZIP payload overlaps central directory"
            );
            physical.push((
                local_offset,
                Record {
                    name: name.to_owned(),
                    data: data_start..data_end,
                    expanded: expanded as u64,
                    crc,
                    method,
                },
            ));
        }
        ensure!(cursor == end, "unaccounted ZIP central data");
        physical.sort_by_key(|(offset, _)| *offset);
        let mut next = 0;
        for (offset, record) in &physical {
            ensure!(
                *offset == next,
                "overlapping or unaccounted ZIP physical records"
            );
            next = record.data.end;
        }
        ensure!(
            next == central_start,
            "unaccounted bytes before ZIP central directory"
        );
        Ok(Self {
            bytes,
            records: physical.into_iter().map(|(_, record)| record).collect(),
        })
    }

    /// Output can be partial on failure. Callers must retain private staging
    /// until this and their payload digest validation have both succeeded.
    pub fn copy(&mut self, name: &str, writer: &mut impl Write) -> Result<u64> {
        let record = self
            .records
            .iter()
            .find(|entry| entry.name == name)
            .context("ZIP member is absent")?;
        let compressed = &self.bytes[record.data.clone()];
        let mut crc = crc32fast::Hasher::new();
        let mut total = 0u64;
        let mut emit = |bytes: &[u8]| -> Result<()> {
            total = total
                .checked_add(bytes.len() as u64)
                .context("ZIP output overflow")?;
            ensure!(total <= record.expanded, "ZIP output exceeds declared size");
            crc.update(bytes);
            writer
                .write_all(bytes)
                .context("write decoded ZIP member")?;
            Ok(())
        };
        if record.method == 0 {
            for chunk in compressed.chunks(8192) {
                emit(chunk)?;
            }
        } else {
            let mut decoder = Decompress::new(false);
            let mut output = [0u8; 8192];
            loop {
                let before_in = decoder.total_in();
                let before_out = decoder.total_out();
                let status = decoder
                    .decompress(
                        &compressed[before_in as usize..],
                        &mut output,
                        FlushDecompress::None,
                    )
                    .context("invalid ZIP deflate stream")?;
                let produced = (decoder.total_out() - before_out) as usize;
                emit(&output[..produced])?;
                if status == Status::StreamEnd {
                    ensure!(
                        decoder.total_in() == compressed.len() as u64,
                        "trailing bytes in ZIP deflate member"
                    );
                    break;
                }
                ensure!(
                    decoder.total_in() > before_in || produced > 0,
                    "truncated ZIP deflate stream"
                );
            }
        }
        ensure!(total == record.expanded, "ZIP output size mismatch");
        ensure!(crc.finalize() == record.crc, "ZIP member CRC mismatch");
        Ok(total)
    }
}

struct BoundedCursor {
    cursor: Cursor<Vec<u8>>,
    limit: u64,
}

impl Write for BoundedCursor {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.cursor.position().saturating_add(bytes.len() as u64) > self.limit {
            return Err(io::Error::other("compressed ZIP output exceeds limit"));
        }
        self.cursor.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.cursor.flush()
    }
}

impl Seek for BoundedCursor {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let prior = self.cursor.position();
        let next = self.cursor.seek(from)?;
        if next > self.limit {
            self.cursor.set_position(prior);
            return Err(io::Error::other("ZIP seek exceeds output limit"));
        }
        Ok(next)
    }
}

/// Write deterministic classic ZIP bytes with a finite output budget.
pub fn write(members: &[WriteMember<'_>], limits: Limits) -> Result<Vec<u8>> {
    ensure!(
        !members.is_empty() && members.len() <= 256,
        "invalid ZIP inventory size"
    );
    let mut expected = Vec::with_capacity(members.len());
    let mut names = HashSet::new();
    let mut total = 0u64;
    for member in members {
        checked_name(member.name, member.kind)?;
        ensure!(names.insert(member.name), "duplicate ZIP writer member");
        let length = member.bytes.len() as u64;
        ensure!(length < u32::MAX as u64, "ZIP64 output is unsupported");
        total = total
            .checked_add(length)
            .context("ZIP writer expanded size overflow")?;
        ensure!(
            total <= limits.max_expanded_bytes,
            "ZIP writer expanded bytes exceed limit"
        );
        ensure!(
            member.kind != MemberKind::Directory || member.bytes.is_empty(),
            "ZIP directory must be empty"
        );
        let mode = match member.kind {
            MemberKind::Directory => 0o040755,
            MemberKind::File if member.executable => 0o100755,
            MemberKind::File => 0o100644,
        };
        expected.push(MemberSpec {
            name: member.name,
            kind: member.kind,
            max_bytes: length,
            exact_bytes: Some(length),
            unix_mode: Some(mode),
        });
    }
    let output = BoundedCursor {
        cursor: Cursor::new(Vec::new()),
        limit: limits.max_compressed_bytes,
    };
    let mut archive = ::zip::ZipWriter::new(output);
    for (member, spec) in members.iter().zip(&expected) {
        let options = ::zip::write::SimpleFileOptions::default()
            .system(::zip::System::Unix)
            .last_modified_time(::zip::DateTime::default())
            .unix_permissions(spec.unix_mode.unwrap())
            .compression_method(if member.kind == MemberKind::Directory {
                ::zip::CompressionMethod::Stored
            } else {
                ::zip::CompressionMethod::Deflated
            });
        if member.kind == MemberKind::Directory {
            archive.add_directory(member.name, options)?;
        } else {
            archive.start_file(member.name, options)?;
            archive.write_all(member.bytes)?;
        }
    }
    let bytes = archive.finish()?.cursor.into_inner();
    Archive::open(&bytes, &expected, limits).context("ZIP writer emitted unsupported records")?;
    Ok(bytes)
}

#[cfg(test)]
mod tests;
