#!/usr/bin/env python3
"""Create a deterministic, self-contained CellRune MCP binary bundle."""

from __future__ import annotations

import argparse
import gzip
import os
import pathlib
import stat
import tarfile
import time
import zipfile

ERROR_PREFIX = "MCP packaging failed"
MESSAGE_SOURCE_DATE_EPOCH_INVALID = (
    "SOURCE_DATE_EPOCH must be set to a Unix timestamp"
)
MESSAGE_EXECUTABLE_MISSING = "CellRune MCP executable not found under {path}"
MESSAGE_BUNDLE_FILE_MISSING = "required bundle file is missing: {path}"
VERSION = "0.1.0"
NOTICE_NAME = "THIRD_PARTY_LICENSES.md"


def source_date_epoch() -> int:
    raw = os.environ.get("SOURCE_DATE_EPOCH")
    if raw is None or not raw.isdecimal():
        raise RuntimeError(MESSAGE_SOURCE_DATE_EPOCH_INVALID)
    return int(raw)


def locate_binary(value: pathlib.Path) -> pathlib.Path:
    if value.is_file():
        return value
    if value.is_dir():
        for name in ("cellrune-mcp", "cellrune-mcp.exe"):
            candidate = value / name
            if candidate.is_file():
                return candidate
    raise RuntimeError(MESSAGE_EXECUTABLE_MISSING.format(path=value))


def bundle_files(
    repository_root: pathlib.Path,
    binary: pathlib.Path,
    third_party_notice: pathlib.Path,
) -> tuple[tuple[str, pathlib.Path, int], ...]:
    return (
        (binary.name, binary, 0o755),
        ("LICENSE", repository_root / "LICENSE", 0o644),
        (
            NOTICE_NAME,
            third_party_notice,
            0o644,
        ),
    )


def write_tar(
    destination: pathlib.Path,
    prefix: str,
    files: tuple[tuple[str, pathlib.Path, int], ...],
    timestamp: int,
) -> None:
    with destination.open("wb") as output:
        with gzip.GzipFile(
            filename="", mode="wb", fileobj=output, mtime=timestamp
        ) as compressed:
            with tarfile.open(
                fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT
            ) as archive:
                for name, source, mode in files:
                    info = archive.gettarinfo(
                        str(source), arcname=f"{prefix}/{name}"
                    )
                    info.uid = 0
                    info.gid = 0
                    info.uname = ""
                    info.gname = ""
                    info.mode = mode
                    info.mtime = timestamp
                    with source.open("rb") as payload:
                        archive.addfile(info, payload)


def write_zip(
    destination: pathlib.Path,
    prefix: str,
    files: tuple[tuple[str, pathlib.Path, int], ...],
    timestamp: int,
) -> None:
    timestamp_tuple = time.gmtime(max(timestamp, 315532800))[:6]
    with zipfile.ZipFile(
        destination, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9
    ) as archive:
        for name, source, mode in files:
            info = zipfile.ZipInfo(f"{prefix}/{name}", date_time=timestamp_tuple)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = (stat.S_IFREG | mode) << 16
            archive.writestr(info, source.read_bytes())


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True, type=pathlib.Path)
    parser.add_argument("--target", required=True)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    parser.add_argument("--notice", type=pathlib.Path)
    arguments = parser.parse_args()

    repository_root = pathlib.Path(__file__).resolve().parents[2]
    binary = locate_binary(arguments.binary.resolve())
    notice = (
        arguments.notice.resolve()
        if arguments.notice is not None
        else repository_root / "crates/cellrune-mcp" / NOTICE_NAME
    )
    files = bundle_files(repository_root, binary, notice)
    for _, source, _ in files:
        if not source.is_file():
            raise RuntimeError(MESSAGE_BUNDLE_FILE_MISSING.format(path=source))
    arguments.output.mkdir(parents=True, exist_ok=True)
    prefix = f"cellrune-mcp-{VERSION}-{arguments.target}"
    if binary.suffix == ".exe":
        destination = arguments.output / f"{prefix}.zip"
        write_zip(destination, prefix, files, source_date_epoch())
    else:
        destination = arguments.output / f"{prefix}.tar.gz"
        write_tar(destination, prefix, files, source_date_epoch())
    print(destination)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, tarfile.TarError) as error:
        raise SystemExit(f"{ERROR_PREFIX}: {error}") from error
