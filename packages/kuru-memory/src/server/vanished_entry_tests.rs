//! The pre-lease directory scan and record reads race a previous owner
//! generation's supervisor retiring `endpoint.json` under a lease they never
//! waited for. Each hook runs exactly where that retirement can land, so these
//! tests reproduce the race deterministically instead of by timing.

use super::*;
use crate::service::is_not_found;

/// A private memory directory holding only a published endpoint record, and
/// a private record outside it to hard-link into its place.
fn fixture() -> Result<(crate::test_support::TempDir, PathBuf, PathBuf)> {
    let root = crate::test_support::tempdir()?;
    let directory = root.path().join("memory");
    private_directory(&directory).context("create private memory fixture")?;
    let endpoint = Endpoint {
        instance: Uuid::new_v4().to_string(),
        port: 1,
    };
    write_record(&directory.join("endpoint.json"), &endpoint).context("publish endpoint")?;
    // Publication leaves an empty staging directory; remove it so the record
    // is the directory's only entry and nothing listed after it can fail.
    fs::remove_dir(directory.join("staging")).context("remove empty staging")?;
    let outside = root.path().join("outside");
    private_directory(&outside).context("create private outside directory")?;
    let linked = outside.join("record.json");
    write_record(&linked, &endpoint).context("write outside record")?;
    Ok((root, directory, linked))
}

#[test]
fn scan_skips_a_listed_record_retired_before_its_open() -> Result<()> {
    let (_root, directory, _) = fixture()?;
    let record = directory.join("endpoint.json");
    let mut retired = None;
    prepare_directory_then(&directory, false, |name| {
        if name == "endpoint.json" {
            retired = Some(fs::remove_file(&record));
        }
    })?;
    retired.context("the scan never listed endpoint.json")??;
    assert!(!record.exists());
    Ok(())
}

#[test]
fn scan_still_fails_a_listed_record_replaced_by_a_hard_link() -> Result<()> {
    let (_root, directory, linked) = fixture()?;
    let record = directory.join("endpoint.json");
    let mut replaced = None;
    let scanned = prepare_directory_then(&directory, false, |name| {
        if name == "endpoint.json" {
            replaced =
                Some(fs::remove_file(&record).and_then(|()| fs::hard_link(&linked, &record)));
        }
    });
    replaced.context("the scan never listed endpoint.json")??;
    let error = scanned.expect_err("a hard-linked record must fail the privacy scan");
    assert!(!is_not_found(&error), "{error:#}");
    Ok(())
}

#[test]
fn scan_still_fails_a_listed_record_replaced_by_a_directory() -> Result<()> {
    let (_root, directory, _) = fixture()?;
    let record = directory.join("endpoint.json");
    let mut replaced = None;
    let scanned = prepare_directory_then(&directory, false, |name| {
        if name == "endpoint.json" {
            replaced = Some(fs::remove_file(&record).and_then(|()| fs::create_dir(&record)));
        }
    });
    replaced.context("the scan never listed endpoint.json")??;
    let error = scanned.expect_err("a directory in a record's place must fail the scan");
    assert!(!is_not_found(&error), "{error:#}");
    Ok(())
}

/// A vanished entry is tolerated only while the scanned directory itself is
/// still the one being scanned; the record is the only entry, so without that
/// check the scan would end successfully on a directory that has moved.
#[cfg(unix)]
#[test]
fn scan_still_fails_when_the_memory_directory_moves_during_it() -> Result<()> {
    let (root, directory, _) = fixture()?;
    let moved = root.path().join("moved");
    let mut renamed = None;
    let scanned = prepare_directory_then(&directory, false, |name| {
        if name == "endpoint.json" {
            renamed = Some(fs::rename(&directory, &moved));
        }
    });
    renamed.context("the scan never listed endpoint.json")??;
    assert!(
        scanned.is_err(),
        "a scan whose directory moved must not succeed"
    );
    Ok(())
}

#[test]
fn record_read_reports_a_record_retired_after_its_check_as_absent() -> Result<()> {
    let (_root, directory, _) = fixture()?;
    let record = directory.join("endpoint.json");
    let mut retired = None;
    let read = read_record_then::<Endpoint>(&record, || {
        retired = Some(fs::remove_file(&record));
    })?;
    retired.context("the read never reached its open")??;
    assert!(read.is_none());
    Ok(())
}

#[test]
fn record_read_still_fails_a_record_replaced_by_a_hard_link_after_its_check() -> Result<()> {
    let (_root, directory, linked) = fixture()?;
    let record = directory.join("endpoint.json");
    let mut replaced = None;
    let read = read_record_then::<Endpoint>(&record, || {
        replaced = Some(fs::remove_file(&record).and_then(|()| fs::hard_link(&linked, &record)));
    });
    replaced.context("the read never reached its open")??;
    let error = read.expect_err("a hard-linked record must fail its checked read");
    assert!(!is_not_found(&error), "{error:#}");
    Ok(())
}

/// A vanished record reads as absent only while its directory is unchanged.
#[cfg(unix)]
#[test]
fn record_read_still_fails_when_its_directory_moves_after_its_check() -> Result<()> {
    let (root, directory, _) = fixture()?;
    let moved = root.path().join("moved");
    let mut renamed = None;
    let read = read_record_then::<Endpoint>(&directory.join("endpoint.json"), || {
        renamed = Some(fs::rename(&directory, &moved));
    });
    renamed.context("the read never reached its open")??;
    assert!(
        read.is_err(),
        "a record read whose directory moved must not report absence"
    );
    Ok(())
}
