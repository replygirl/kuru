import hashlib
import io
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from install_release import checked_version, install, main, read_asset
from package_release import package


class ReleaseInstallTests(unittest.TestCase):
    target = "aarch64-apple-darwin"

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / "releases"
        self.source.mkdir()
        self.dest = self.root / "bin"
        self.dest.mkdir()
        self.current = self.dest / "kuru"
        self.current.write_text("previous version")
        self.archive = self.source / f"kuru-0.1.0-{self.target}.tar.gz"

    def fixture(
        self,
        name="kuru",
        content=b"#!/bin/sh\necho kuru 0.1.0\n",
        mode=0o755,
        kind=tarfile.REGTYPE,
        duplicate=False,
    ):
        with tarfile.open(self.archive, "w:gz") as stream:
            entry = tarfile.TarInfo(name)
            entry.mode = mode
            entry.type = kind
            entry.linkname = "/tmp/outside"
            entry.size = len(content) if kind == tarfile.REGTYPE else 0
            stream.addfile(entry, io.BytesIO(content))
            if duplicate:
                stream.addfile(entry, io.BytesIO(content))
        digest = hashlib.sha256(self.archive.read_bytes()).hexdigest()
        (self.source / "SHA256SUMS").write_text(f"{digest}  {self.archive.name}\n")

    def run_install(self, **kwargs):
        return install(
            str(self.source), "v0.1.0", self.dest, kwargs.get("target", self.target)
        )

    def assert_unchanged(self):
        self.assertEqual(self.current.read_text(), "previous version")
        self.assertFalse(list(self.dest.glob(".kuru-install-*")))

    def test_valid_release_replaces_executable_and_runs(self):
        self.fixture()
        installed = self.run_install()
        result = subprocess.run(
            [installed, "--version"], capture_output=True, text=True, check=True
        )
        self.assertEqual(result.stdout.strip(), "kuru 0.1.0")
        self.assertFalse(list(self.dest.glob(".kuru-install-*")))

    def test_corruption_preserves_previous_binary(self):
        self.fixture()
        self.archive.write_bytes(b"changed archive")
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            self.run_install()
        self.assert_unchanged()

    def test_archive_traversal_links_and_nonexecutables_preserve_previous(self):
        for name, mode, kind in [
            ("../kuru", 0o755, tarfile.REGTYPE),
            ("/tmp/kuru", 0o755, tarfile.REGTYPE),
            ("kuru", 0o755, tarfile.SYMTYPE),
            ("kuru", 0o644, tarfile.REGTYPE),
        ]:
            with self.subTest(name=name, kind=kind, mode=mode):
                self.fixture(name=name, mode=mode, kind=kind)
                with self.assertRaises(ValueError):
                    self.run_install()
                self.assert_unchanged()

    def test_compressed_bomb_rejected_before_parsing_tar_metadata(self):
        self.fixture(content=b"x" * 4096)
        self.assertLess(self.archive.stat().st_size, 2048)
        with patch("install_release.MAX_ARCHIVE_BYTES", 2048):
            with self.assertRaisesRegex(ValueError, "expanded release archive"):
                self.run_install()
        self.assert_unchanged()

    def test_duplicate_member_is_rejected(self):
        self.fixture(duplicate=True)
        with self.assertRaisesRegex(ValueError, "exactly one"):
            self.run_install()
        self.assert_unchanged()

    def test_duplicate_checksum_is_rejected(self):
        self.fixture()
        manifest = self.source / "SHA256SUMS"
        manifest.write_text(manifest.read_text() * 2)
        with self.assertRaisesRegex(ValueError, "exactly once"):
            self.run_install()
        self.assert_unchanged()

    def test_wrong_platform_and_unsafe_versions_rejected(self):
        with self.assertRaisesRegex(ValueError, "unsupported platform"):
            self.run_install(target="unknown")
        for version in ["latest", "../../x", "1.0", "1.0.0;id", "v1.0.0/evil"]:
            with self.subTest(version=version), self.assertRaises(ValueError):
                checked_version(version)
        self.assertEqual(checked_version("v1.2.3-rc.1"), "1.2.3-rc.1")
        self.assert_unchanged()

    def test_destination_symlink_cannot_replace_target(self):
        self.fixture()
        outside = self.root / "outside"
        outside.write_text("outside")
        self.current.unlink()
        self.current.symlink_to(outside)
        with self.assertRaisesRegex(ValueError, "symlink"):
            self.run_install()
        self.assertEqual(outside.read_text(), "outside")

    def test_remote_transport_constraints_and_size_limit(self):
        for base in [
            "http://example.invalid",
            "https://user:pass@example.invalid",
            "ftp://example.invalid",
            "https://example.invalid/#fragment",
        ]:
            with self.subTest(base=base), self.assertRaises(ValueError):
                read_asset(base, "SHA256SUMS", 10)
        (self.source / "large").write_bytes(b"x" * 11)
        with self.assertRaisesRegex(ValueError, "size limit"):
            read_asset(str(self.source), "large", 10)

    def test_installer_cli_reports_actionable_failure(self):
        self.fixture()
        self.archive.write_bytes(b"bad")
        self.assertEqual(
            main(
                [
                    "--release-base",
                    str(self.source),
                    "--version",
                    "0.1.0",
                    "--install-dir",
                    str(self.dest),
                    "--target",
                    self.target,
                ]
            ),
            1,
        )
        self.assert_unchanged()

    def test_real_packager_to_installer_roundtrip(self):
        binary = self.root / "fixture-kuru"
        binary.write_text("#!/bin/sh\necho kuru 0.2.0\n")
        binary.chmod(0o755)
        archive = package(binary, self.target, "0.2.0", self.source)
        repeated = package(binary, self.target, "0.2.0", self.root / "repeated")
        self.assertEqual(archive.read_bytes(), repeated.read_bytes())
        (self.source / "SHA256SUMS").write_text(
            (self.source / f"{archive.name}.sha256").read_text()
        )
        installed = install(str(self.source), "0.2.0", self.dest, self.target)
        self.assertEqual(
            subprocess.check_output([installed], text=True).strip(), "kuru 0.2.0"
        )


if __name__ == "__main__":
    unittest.main()
