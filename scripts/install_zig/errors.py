"""Error codes for the repository-owned Zig installer.

The installer only fails through these codes so a workflow log line can be
matched mechanically and never through an unexpected traceback.
"""

from __future__ import annotations

from dataclasses import dataclass

DOWNLOAD_FAILED = "zig_install.download_failed"
CHECKSUM_MISMATCH = "zig_install.checksum_mismatch"
UNSAFE_ARCHIVE_MEMBER = "zig_install.unsafe_archive_member"
UNSUPPORTED_PLATFORM = "zig_install.unsupported_platform"
UNSAFE_INSTALL_ROOT = "zig_install.unsafe_install_root"
VERSION_MISMATCH = "zig_install.version_mismatch"
MISSING_TOOLCHAIN = "zig_install.missing_toolchain"


@dataclass(frozen=True)
class InstallerError(Exception):
    """A machine-readable installer failure."""

    code: str
    message: str
