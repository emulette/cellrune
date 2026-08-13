"""Repository-owned Zig 0.14.1 installer for Linux x86_64 CI runners.

This replaces the third-party ``mlugg/setup-zig`` action with a pinned,
checksum-verified installer that downloads the official Zig archive, extracts
it without path traversal, verifies the exact toolchain version, and appends
its ``bin`` directory to ``GITHUB_PATH``.

The installer only installs under ``RUNNER_TEMP`` or ``RUNNER_TOOL_CACHE`` so a
local or unexpected environment cannot silently widen the artifact matrix.
"""

from __future__ import annotations

import hashlib
import os
import pathlib
import shutil
import subprocess
import sys
import tarfile
import time
import urllib.request

from errors import (
    CHECKSUM_MISMATCH,
    DOWNLOAD_FAILED,
    MISSING_TOOLCHAIN,
    UNSAFE_ARCHIVE_MEMBER,
    UNSAFE_INSTALL_ROOT,
    UNSUPPORTED_PLATFORM,
    VERSION_MISMATCH,
    InstallerError,
)

ZIG_VERSION = "0.14.1"
PLATFORM = "x86_64-linux"
ARCHIVE_NAME = f"zig-{PLATFORM}-{ZIG_VERSION}.tar.xz"
ARCHIVE_URL = f"https://ziglang.org/download/{ZIG_VERSION}/{ARCHIVE_NAME}"
ARCHIVE_SHA256 = "24aeeec8af16c381934a6cd7d95c807a8cb2cf7df9fa40d359aa884195c4716c"
DOWNLOAD_ATTEMPTS = 3
DOWNLOAD_RETRY_DELAY_SECONDS = 2.0


def run() -> None:
    if sys.platform != "linux" or os.environ.get("RUNNER_ARCH") not in (None, "X64"):
        raise InstallerError(
            UNSUPPORTED_PLATFORM,
            "this installer only supports linux x86_64 CI runners",
        )

    install_root = install_root_path()
    archive_path = download_archive()
    extract_dir = extract_archive(archive_path, install_root)
    verify_toolchain(extract_dir)
    publish_toolchain(extract_dir)


def install_root_path() -> pathlib.Path:
    for variable in ("RUNNER_TEMP", "RUNNER_TOOL_CACHE"):
        value = os.environ.get(variable)
        if value:
            root = pathlib.Path(value)
            root.mkdir(parents=True, exist_ok=True)
            return root
    raise InstallerError(
        UNSAFE_INSTALL_ROOT,
        "RUNNER_TEMP and RUNNER_TOOL_CACHE are both unset; refusing to install elsewhere",
    )


def download_archive() -> pathlib.Path:
    destination = install_root_path() / ARCHIVE_NAME
    last_error: Exception | None = None
    for attempt in range(1, DOWNLOAD_ATTEMPTS + 1):
        try:
            with urllib.request.urlopen(ARCHIVE_URL, timeout=120) as response:
                data = response.read()
        except Exception as error:  # noqa: BLE001 - retried below, then failed with a stable code
            last_error = error
            if attempt < DOWNLOAD_ATTEMPTS:
                time.sleep(DOWNLOAD_RETRY_DELAY_SECONDS)
            continue
        digest = hashlib.sha256(data).hexdigest()
        if digest != ARCHIVE_SHA256:
            raise InstallerError(
                CHECKSUM_MISMATCH,
                f"expected {ARCHIVE_SHA256}, got {digest}",
            )
        destination.write_bytes(data)
        return destination
    raise InstallerError(DOWNLOAD_FAILED, str(last_error))


def extract_archive(archive_path: pathlib.Path, install_root: pathlib.Path) -> pathlib.Path:
    destination = install_root / f"zig-{ZIG_VERSION}"
    if destination.exists():
        shutil.rmtree(destination)
    destination.mkdir(parents=True, exist_ok=True)

    with tarfile.open(archive_path, "r:xz") as archive:
        for member in archive.getmembers():
            if not is_safe_member(member.name):
                raise InstallerError(
                    UNSAFE_ARCHIVE_MEMBER,
                    f"archive member escapes the install root: {member.name}",
                )
        archive.extractall(destination, filter="data")

    extracted = destination / ARCHIVE_NAME.removesuffix(".tar.xz")
    if not extracted.is_dir():
        raise InstallerError(
            UNSAFE_ARCHIVE_MEMBER,
            f"archive did not contain the expected top-level directory {extracted.name}",
        )
    return extracted


def is_safe_member(name: str) -> bool:
    normalized = pathlib.PurePosixPath(name)
    if normalized.is_absolute():
        return False
    return ".." not in normalized.parts


def verify_toolchain(extract_dir: pathlib.Path) -> None:
    binary = extract_dir / "zig"
    if not binary.is_file():
        raise InstallerError(MISSING_TOOLCHAIN, f"{binary} is missing")
    result = subprocess.run(
        [str(binary), "version"],
        check=False,
        capture_output=True,
        text=True,
    )
    reported = result.stdout.strip()
    if result.returncode != 0 or reported != ZIG_VERSION:
        raise InstallerError(
            VERSION_MISMATCH,
            f"expected zig {ZIG_VERSION}, got {reported or result.stderr.strip()}",
        )


def publish_toolchain(extract_dir: pathlib.Path) -> None:
    github_path = os.environ.get("GITHUB_PATH")
    if not github_path:
        raise InstallerError(
            UNSAFE_INSTALL_ROOT,
            "GITHUB_PATH is unset; cannot publish the toolchain to the workflow PATH",
        )
    with open(github_path, "a", encoding="utf-8") as stream:
        stream.write(f"{extract_dir}\n")


if __name__ == "__main__":
    try:
        run()
    except InstallerError as error:
        print(f"error: {error.code}: {error.message}", file=sys.stderr)
        sys.exit(1)
