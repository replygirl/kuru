use super::*;

const LIMITS: Limits = Limits {
    max_compressed_bytes: 1_000_000,
    max_expanded_bytes: 1_000_000,
    allow_ntfs_timestamps: false,
};

fn spec(name: &str, kind: MemberKind, length: u64) -> MemberSpec<'_> {
    MemberSpec {
        name,
        kind,
        max_bytes: length,
        exact_bytes: Some(length),
        unix_mode: None,
    }
}

// Generate independent fixtures through upstream ZIP's general writer. This
// permits valid ZIP features deliberately outside Kuru's accepted subset.
fn fixture(method: ::zip::CompressionMethod, data: &[u8]) -> Vec<u8> {
    let mut zip = ::zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file(
        "payload",
        ::zip::write::SimpleFileOptions::default()
            .system(::zip::System::Unix)
            .compression_method(method)
            .unix_permissions(0o644),
    )
    .unwrap();
    zip.write_all(data).unwrap();
    zip.finish().unwrap().into_inner()
}

fn central(bytes: &[u8]) -> usize {
    u32_at(bytes, bytes.len() - 6).unwrap() as usize
}

fn put16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn open_one(bytes: &[u8], size: u64) -> Result<Archive<'_>> {
    Archive::open(bytes, &[spec("payload", MemberKind::File, size)], LIMITS)
}

#[test]
fn deterministic_writer_preserves_inventory_bytes_and_permissions() {
    let payload = b"actual executable bytes\0\xff".repeat(1300);
    let members = [
        WriteMember {
            name: "bin/",
            kind: MemberKind::Directory,
            bytes: b"",
            executable: false,
        },
        WriteMember {
            name: "bin/kuru.exe",
            kind: MemberKind::File,
            bytes: &payload,
            executable: true,
        },
        WriteMember {
            name: "LICENSE",
            kind: MemberKind::File,
            bytes: b"terms",
            executable: false,
        },
    ];
    let bytes = write(&members, LIMITS).unwrap();
    assert_eq!(bytes, write(&members, LIMITS).unwrap());
    let expected = [
        MemberSpec {
            unix_mode: Some(0o040755),
            ..spec("bin/", MemberKind::Directory, 0)
        },
        MemberSpec {
            unix_mode: Some(0o100755),
            ..spec("bin/kuru.exe", MemberKind::File, payload.len() as u64)
        },
        MemberSpec {
            unix_mode: Some(0o100644),
            ..spec("LICENSE", MemberKind::File, 5)
        },
    ];
    let mut archive = Archive::open(&bytes, &expected, LIMITS).unwrap();
    for member in &members {
        let mut output = Vec::new();
        assert_eq!(
            archive.copy(member.name, &mut output).unwrap(),
            member.bytes.len() as u64
        );
        assert_eq!(output, member.bytes);
    }
    assert!(archive.copy("missing", &mut Vec::new()).is_err());
    let mut ordinary_zip = ::zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
    assert_eq!(
        ordinary_zip.by_name("bin/kuru.exe").unwrap().unix_mode(),
        Some(0o100755)
    );
}

#[test]
fn stored_deflated_and_empty_members_decode_without_assuming_writer_layout() {
    for method in [
        ::zip::CompressionMethod::Stored,
        ::zip::CompressionMethod::Deflated,
    ] {
        for data in [vec![], b"hello\0world".repeat(2000)] {
            let bytes = fixture(method, &data);
            let mut output = Vec::new();
            open_one(&bytes, data.len() as u64)
                .unwrap()
                .copy("payload", &mut output)
                .unwrap();
            assert_eq!(output, data);
        }
    }
}

#[test]
fn every_truncated_prefix_and_appended_record_is_rejected() {
    let bytes = fixture(::zip::CompressionMethod::Stored, b"content");
    for end in 0..bytes.len() {
        assert!(
            open_one(&bytes[..end], 7).is_err(),
            "accepted truncated length {end}"
        );
    }
    for suffix in [b"\0".as_slice(), b"PK\x05\x06", bytes.as_slice()] {
        let mut altered = bytes.clone();
        altered.extend_from_slice(suffix);
        assert!(open_one(&altered, 7).is_err());
    }
}

#[test]
fn contradictory_flags_links_descriptors_versions_and_offsets_are_rejected() {
    let original = fixture(::zip::CompressionMethod::Stored, b"content");
    let cd = central(&original);
    let footer = original.len() - 22;
    let mutations: &[(usize, &[u8])] = &[
        (cd, b"BAD!"),
        (cd + 5, &[0]),
        (cd + 6, &[45, 0]),
        (cd + 8, &[1, 0]),
        (cd + 8, &[8, 0]),
        (cd + 8, &[0, 8]),
        (cd + 10, &[12, 0]),
        (cd + 20, &[255; 4]),
        (cd + 24, &[255; 4]),
        (cd + 32, &[1, 0]),
        (cd + 34, &[1, 0]),
        (cd + 36, &[1, 0]),
        (cd + 38, &[0x20, 0, 0xff, 0xa1]), // symlink, even with a normal name
        (cd + 38, &[0x20, 0, 0xa4, 0x89]), // setuid regular file
        (cd + 38, &[0x40, 0, 0xa4, 0x81]), // unsupported DOS attribute
        (cd + 42, &[255; 4]),
        (cd + 46, b"Payload"),
        (0, b"BAD!"),
        (4, &[45, 0]),
        (26, &[6, 0]),
        (28, &[1, 0]),
        (30, b"Payload"),
        (footer + 4, &[1, 0]),
        (footer + 6, &[1, 0]),
        (footer + 8, &[0, 0]),
        (footer + 10, &[0, 0]),
        (footer + 12, &[0; 4]),
        (footer + 16, &[0; 4]),
        (footer + 20, &[1, 0]),
    ];
    for &(offset, replacement) in mutations {
        let mut bytes = original.clone();
        bytes[offset..offset + replacement.len()].copy_from_slice(replacement);
        assert!(
            open_one(&bytes, 7).is_err(),
            "accepted changed field at {offset}: {replacement:?}"
        );
    }
}

#[test]
fn hidden_duplicate_physical_local_records_are_not_lost_by_name_lookup() {
    let original = fixture(::zip::CompressionMethod::Stored, b"content");
    let cd = central(&original);
    // Keep one central member, but precede its local record with an unindexed
    // physical duplicate. ZIP readers which only follow the index accept this.
    let mut bytes = original[..cd].to_vec();
    bytes.extend_from_slice(&original);
    let footer = bytes.len() - 22;
    put32(&mut bytes, footer + 16, (cd * 2) as u32);
    put32(&mut bytes, cd * 2 + 42, cd as u32);
    assert!(::zip::ZipArchive::new(Cursor::new(&bytes)).is_ok());
    assert!(
        open_one(&bytes, 7)
            .unwrap_err()
            .to_string()
            .contains("physical records")
    );

    let members = [
        WriteMember {
            name: "first",
            kind: MemberKind::File,
            bytes: b"x",
            executable: false,
        },
        WriteMember {
            name: "other",
            kind: MemberKind::File,
            bytes: b"y",
            executable: false,
        },
    ];
    let original = write(&members, LIMITS).unwrap();
    let cd = central(&original);
    let second = cd + 46 + 5;
    let expected = [
        spec("first", MemberKind::File, 1),
        spec("other", MemberKind::File, 1),
    ];
    let mut bytes = original.clone();
    bytes[second + 46..second + 51].copy_from_slice(b"first");
    assert!(
        Archive::open(&bytes, &expected, LIMITS)
            .unwrap_err()
            .to_string()
            .contains("duplicate physical")
    );
    let mut bytes = original;
    put32(&mut bytes, second + 42, 0);
    assert!(
        Archive::open(&bytes, &expected, LIMITS)
            .unwrap_err()
            .to_string()
            .contains("duplicate ZIP local")
    );
}

#[test]
fn only_audited_central_ntfs_timestamps_are_accepted() {
    let mut bytes = fixture(::zip::CompressionMethod::Stored, b"content");
    let cd = central(&bytes);
    // Match the actual upstream 7-Zip attributes, including the low 0x8000
    // marker. Payload type still comes exclusively from the checked Unix mode.
    put32(&mut bytes, cd + 38, 0o100644 << 16 | 0x8020);
    let mut ntfs = vec![0; 36];
    put16(&mut ntfs, 0, 0x000a);
    put16(&mut ntfs, 2, 32);
    put16(&mut ntfs, 8, 1);
    put16(&mut ntfs, 10, 24);
    // Three FILETIME values are metadata only, never interpreted as authority.
    ntfs[12..].fill(0x55);
    let extra_at = bytes.len() - 22;
    bytes.splice(extra_at..extra_at, ntfs);
    put16(&mut bytes, cd + 30, 36);
    let footer = bytes.len() - 22;
    let size = u32_at(&bytes, footer + 12).unwrap();
    put32(&mut bytes, footer + 12, size + 36);
    let policy = [spec("payload", MemberKind::File, 7)];
    let allowed = Limits {
        allow_ntfs_timestamps: true,
        ..LIMITS
    };
    let mut archive = Archive::open(&bytes, &policy, allowed).unwrap();
    let mut output = Vec::new();
    archive.copy("payload", &mut output).unwrap();
    assert_eq!(output, b"content");
    assert!(Archive::open(&bytes, &policy, LIMITS).is_err());
    for offset in [0, 2, 4, 8, 10] {
        let mut altered = bytes.clone();
        altered[extra_at + offset] ^= 1;
        assert!(Archive::open(&altered, &policy, allowed).is_err());
    }
}

#[test]
fn limits_names_and_exact_inventory_apply_before_any_payload_is_written() {
    let bytes = fixture(::zip::CompressionMethod::Stored, b"content");
    let expected = [spec("payload", MemberKind::File, 7)];
    assert!(
        Archive::open(
            &bytes,
            &expected,
            Limits {
                max_compressed_bytes: bytes.len() as u64 - 1,
                ..LIMITS
            }
        )
        .is_err()
    );
    assert!(
        Archive::open(
            &bytes,
            &expected,
            Limits {
                max_expanded_bytes: 6,
                ..LIMITS
            }
        )
        .is_err()
    );
    assert!(open_one(&bytes, 6).is_err());
    assert!(open_one(&bytes, 8).is_err());
    assert!(Archive::open(&bytes, &[], LIMITS).is_err());
    assert!(Archive::open(&bytes, &[expected[0], expected[0]], LIMITS).is_err());
    for name in [
        "",
        "../payload",
        "/payload",
        "./payload",
        "x//payload",
        "x\\payload",
        "C:payload",
        "payload\n",
        "páyload",
        "payload/",
    ] {
        assert!(
            Archive::open(&bytes, &[spec(name, MemberKind::File, 7)], LIMITS).is_err(),
            "accepted {name:?}"
        );
    }
    assert!(
        Archive::open(
            &bytes,
            &[MemberSpec {
                unix_mode: Some(0o100755),
                ..expected[0]
            }],
            LIMITS
        )
        .is_err()
    );
}

#[test]
fn damaged_crc_streams_and_false_expanded_sizes_cannot_publish_decoded_bytes() {
    for method in [
        ::zip::CompressionMethod::Stored,
        ::zip::CompressionMethod::Deflated,
    ] {
        let mut bytes = fixture(method, b"content");
        let cd = central(&bytes);
        put32(&mut bytes, 14, 0);
        put32(&mut bytes, cd + 16, 0);
        assert!(
            open_one(&bytes, 7)
                .unwrap()
                .copy("payload", &mut Vec::new())
                .unwrap_err()
                .to_string()
                .contains("CRC")
        );
    }
    let original = fixture(::zip::CompressionMethod::Deflated, &vec![b'x'; 20_000]);
    let cd = central(&original);
    for declared in [1, 20_001] {
        let mut bytes = original.clone();
        put32(&mut bytes, 22, declared);
        put32(&mut bytes, cd + 24, declared);
        let mut output = Vec::new();
        assert!(
            open_one(&bytes, declared as u64)
                .unwrap()
                .copy("payload", &mut output)
                .is_err()
        );
        assert!(output.len() <= declared as usize);
    }
    let mut invalid = original.clone();
    invalid[37] = 0xff; // reserved deflate block type
    assert!(
        open_one(&invalid, 20_000)
            .unwrap()
            .copy("payload", &mut Vec::new())
            .is_err()
    );

    // Shorten or extend the actual compressed stream while consistently moving
    // its central directory and updating both metadata copies.
    for truncate in [false, true] {
        let mut bytes = original.clone();
        let compressed = u32_at(&bytes, 18).unwrap();
        let (new_cd, new_length) = if truncate {
            bytes.remove(cd - 1);
            (cd - 1, compressed - 1)
        } else {
            bytes.insert(cd, 0);
            (cd + 1, compressed + 1)
        };
        let footer = bytes.len() - 22;
        put32(&mut bytes, footer + 16, new_cd as u32);
        put32(&mut bytes, 18, new_length);
        put32(&mut bytes, new_cd + 20, new_length);
        assert!(
            open_one(&bytes, 20_000)
                .unwrap()
                .copy("payload", &mut Vec::new())
                .is_err()
        );
    }
}

#[test]
fn writer_errors_and_both_output_budgets_are_observable() {
    struct FailingWriter;
    impl Write for FailingWriter {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("fixture disk full"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let bytes = fixture(::zip::CompressionMethod::Stored, b"content");
    let error = open_one(&bytes, 7)
        .unwrap()
        .copy("payload", &mut FailingWriter)
        .unwrap_err();
    assert!(format!("{error:#}").contains("fixture disk full"));
    let member = WriteMember {
        name: "payload",
        kind: MemberKind::File,
        bytes: b"content",
        executable: false,
    };
    assert!(
        write(
            &[member],
            Limits {
                max_compressed_bytes: 30,
                ..LIMITS
            }
        )
        .is_err()
    );
    let member = WriteMember {
        name: "payload",
        kind: MemberKind::File,
        bytes: b"content",
        executable: false,
    };
    assert!(
        write(
            &[member],
            Limits {
                max_expanded_bytes: 6,
                ..LIMITS
            }
        )
        .is_err()
    );
    assert!(write(&[], LIMITS).is_err());
    assert!(
        write(
            &[WriteMember {
                name: "dir/",
                kind: MemberKind::Directory,
                bytes: b"hidden",
                executable: false
            }],
            LIMITS
        )
        .is_err()
    );
    assert!(
        write(
            &[
                WriteMember {
                    name: "same",
                    kind: MemberKind::File,
                    bytes: b"",
                    executable: false
                },
                WriteMember {
                    name: "same",
                    kind: MemberKind::File,
                    bytes: b"",
                    executable: false
                },
            ],
            LIMITS
        )
        .is_err()
    );
}
