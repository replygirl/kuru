#!/usr/bin/env python3
"""Install one explicitly versioned, checksum-verified Kuru release."""

import argparse
import gzip
import hashlib
import io
import os
import platform
import re
import shutil
import stat
import sys
import tarfile
import tempfile
from pathlib import Path
from urllib.parse import urlparse
from urllib.request import urlopen

MAX_ARCHIVE_BYTES = 128 * 1024 * 1024
TARGETS = {
    ("Darwin", "arm64"): "aarch64-apple-darwin",
    ("Darwin", "x86_64"): "x86_64-apple-darwin",
    ("Linux", "aarch64"): "aarch64-unknown-linux-gnu",
    ("Linux", "x86_64"): "x86_64-unknown-linux-gnu",
}


def checked_version(value):
    value = value.removeprefix("v")
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?", value):
        raise ValueError(
            "version must be an explicit semantic version, for example 0.1.0"
        )
    return value


def read_asset(base, name, limit):
    """Only HTTPS remote sources or explicit local fixture/release directories."""
    parsed = urlparse(base)
    if parsed.scheme:
        if (
            parsed.scheme != "https"
            or not parsed.netloc
            or parsed.username
            or parsed.password
        ):
            raise ValueError("release base must be HTTPS without embedded credentials")
        if parsed.query or parsed.fragment:
            raise ValueError("release base must not include a query or fragment")
        with urlopen(base.rstrip("/") + "/" + name, timeout=60) as response:
            if urlparse(response.url).scheme != "https":
                raise ValueError("release redirect downgraded HTTPS")
            content = response.read(limit + 1)
    else:
        with (Path(base) / name).open("rb") as stream:
            content = stream.read(limit + 1)
    if len(content) > limit:
        raise ValueError("release asset exceeds size limit")
    return content


def expected_digest(manifest, filename):
    matches = []
    for line in manifest.decode("utf-8").splitlines():
        match = re.fullmatch(r"([a-fA-F0-9]{64}) [ *](\S+)", line)
        if match and match[2] == filename:
            matches.append(match[1].lower())
    if len(matches) != 1:
        raise ValueError("checksum manifest must name the release archive exactly once")
    return matches[0]


def install(base, version, destination, target=None):
    version = checked_version(version)
    target = target or TARGETS.get((platform.system(), platform.machine()))
    if target not in TARGETS.values():
        raise ValueError("unsupported platform; build from source with Rust")
    archive_name = f"kuru-{version}-{target}.tar.gz"
    digest = expected_digest(read_asset(base, "SHA256SUMS", 64 * 1024), archive_name)
    archive = read_asset(base, archive_name, MAX_ARCHIVE_BYTES)
    if hashlib.sha256(archive).hexdigest() != digest:
        raise ValueError(
            "release archive checksum mismatch; existing executable unchanged"
        )

    destination = Path(destination).expanduser().resolve()
    destination.mkdir(parents=True, exist_ok=True)
    executable = destination / "kuru"
    if executable.is_symlink() or (executable.exists() and not executable.is_file()):
        raise ValueError(
            "destination kuru must be a regular file, not a symlink or directory"
        )
    with tempfile.TemporaryDirectory(
        prefix=".kuru-install-", dir=destination
    ) as temporary:
        staging = Path(temporary)
        archive_path = staging / archive_name
        # Bound expanded bytes before tarfile parses headers (including PAX
        # metadata), so a small compressed archive cannot exhaust memory or CPU.
        with gzip.GzipFile(fileobj=io.BytesIO(archive)) as compressed:
            expanded = compressed.read(MAX_ARCHIVE_BYTES + 1)
        if len(expanded) > MAX_ARCHIVE_BYTES:
            raise ValueError("expanded release archive exceeds size limit")
        archive_path.write_bytes(expanded)
        with tarfile.open(archive_path, "r:") as release:
            members = release.getmembers()
            allowed = {"kuru", "LICENSE", "README.md"}
            if not members or len(members) > len(allowed):
                raise ValueError("release archive has unexpected entries")
            names = [entry.name for entry in members]
            if len(set(names)) != len(names) or "kuru" not in names:
                raise ValueError(
                    "release archive must contain exactly one kuru executable"
                )
            for entry in members:
                if (
                    entry.name not in allowed
                    or not entry.isfile()
                    or entry.size > MAX_ARCHIVE_BYTES
                ):
                    raise ValueError(
                        "release archive contains unsafe paths, links or oversized entries"
                    )
            binary_info = release.getmember("kuru")
            if not binary_info.mode & stat.S_IXUSR or binary_info.size == 0:
                raise ValueError("release kuru entry is not an executable")
            source = release.extractfile(binary_info)
            if source is None:
                raise ValueError("release executable is unreadable")
            candidate = staging / "kuru"
            with candidate.open("xb") as output:
                shutil.copyfileobj(source, output)
                output.flush()
                os.fsync(output.fileno())
            candidate.chmod(0o755)
        os.replace(candidate, executable)
    return executable


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--release-base",
        default=os.environ.get("KURU_RELEASE_BASE"),
        help="HTTPS directory or local directory containing archives and SHA256SUMS",
    )
    parser.add_argument("--version", required=True, help="explicit release version")
    parser.add_argument(
        "--install-dir",
        default=os.environ.get("KURU_INSTALL_DIR", str(Path.home() / ".local/bin")),
    )
    parser.add_argument("--target", choices=sorted(set(TARGETS.values())))
    args = parser.parse_args(argv)
    if not args.release_base:
        parser.error(
            "--release-base or KURU_RELEASE_BASE is required until a public release repository is configured"
        )
    try:
        path = install(args.release_base, args.version, args.install_dir, args.target)
    except (OSError, ValueError, tarfile.TarError) as error:
        print(f"kuru install: {error}", file=sys.stderr)
        return 1
    print(f"Installed Kuru {checked_version(args.version)} at {path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
