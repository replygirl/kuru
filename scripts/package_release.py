#!/usr/bin/env python3
"""Package a Kuru binary and documentation with a SHA-256 checksum."""

import argparse
import gzip
import hashlib
import tarfile
from pathlib import Path

from install_release import TARGETS, checked_version


def package(binary, target, version, output):
    binary = Path(binary)
    if binary.is_symlink() or not binary.is_file() or not binary.stat().st_mode & 0o111:
        raise ValueError("binary must be an existing executable file")
    if target not in TARGETS.values():
        raise ValueError("unsupported release target")
    version = checked_version(version)
    output = Path(output)
    output.mkdir(parents=True, exist_ok=True)
    archive = output / f"kuru-{version}-{target}.tar.gz"
    root = Path(__file__).resolve().parent.parent
    with archive.open("wb") as raw:
        with gzip.GzipFile(filename="", fileobj=raw, mode="wb", mtime=0) as compressed:
            with tarfile.open(
                fileobj=compressed, mode="w", format=tarfile.USTAR_FORMAT
            ) as stream:
                for path, name in [
                    (binary, "kuru"),
                    (root / "LICENSE", "LICENSE"),
                    (root / "README.md", "README.md"),
                ]:
                    info = stream.gettarinfo(path, arcname=name)
                    info.uid = info.gid = 0
                    info.uname = info.gname = ""
                    info.mtime = 0
                    info.mode = 0o755 if name == "kuru" else 0o644
                    with path.open("rb") as source:
                        stream.addfile(info, source)
    checksum = f"{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n"
    (output / f"{archive.name}.sha256").write_text(checksum)
    return archive


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True)
    parser.add_argument(
        "--target", required=True, choices=sorted(set(TARGETS.values()))
    )
    parser.add_argument("--version", required=True)
    parser.add_argument("--output", default="dist")
    args = parser.parse_args()
    print(package(args.binary, args.target, args.version, args.output))


if __name__ == "__main__":
    main()
