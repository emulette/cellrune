#!/usr/bin/env python3
"""Regression tests for release archive entry-type validation."""

from __future__ import annotations

import io
import stat
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path

from verify_release_artifacts import (
    EXPECTED_VERSION,
    Entry,
    archive_entries,
    validate_mcp,
    validate_wheel_platform_tag,
)


class ArchiveEntryTests(unittest.TestCase):
    def test_tar_symlink_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            archive_path = Path(temporary) / "artifact.tar.gz"
            with tarfile.open(archive_path, "w:gz") as archive:
                link = tarfile.TarInfo("package/link")
                link.type = tarfile.SYMTYPE
                link.linkname = "target"
                archive.addfile(link)

            with self.assertRaisesRegex(RuntimeError, r"^archive\.special_entry:"):
                archive_entries(archive_path)

    def test_zip_symlink_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            archive_path = Path(temporary) / "artifact.zip"
            with zipfile.ZipFile(archive_path, "w") as archive:
                link = zipfile.ZipInfo("package/link")
                link.create_system = 3
                link.external_attr = (stat.S_IFLNK | 0o777) << 16
                archive.writestr(link, "target")

            with self.assertRaisesRegex(RuntimeError, r"^archive\.special_entry:"):
                archive_entries(archive_path)

    def test_unsafe_directory_path_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            archive_path = Path(temporary) / "artifact.tar.gz"
            with tarfile.open(archive_path, "w:gz") as archive:
                directory = tarfile.TarInfo("../outside")
                directory.type = tarfile.DIRTYPE
                archive.addfile(directory)

            with self.assertRaisesRegex(RuntimeError, r"^archive\.path_unsafe:"):
                archive_entries(archive_path)

    def test_windows_shaped_path_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            archive_path = Path(temporary) / "artifact.tar.gz"
            payload = b"release"
            with tarfile.open(archive_path, "w:gz") as archive:
                member = tarfile.TarInfo(r"..\outside")
                member.size = len(payload)
                archive.addfile(member, io.BytesIO(payload))

            with self.assertRaisesRegex(RuntimeError, r"^archive\.path_unsafe:"):
                archive_entries(archive_path)

    def test_duplicate_member_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            archive_path = Path(temporary) / "artifact.tar.gz"
            payload = b"release"
            with tarfile.open(archive_path, "w:gz") as archive:
                for _ in range(2):
                    member = tarfile.TarInfo("package/file.txt")
                    member.size = len(payload)
                    archive.addfile(member, io.BytesIO(payload))

            with self.assertRaisesRegex(
                RuntimeError,
                r"^archive\.duplicate_member:",
            ):
                archive_entries(archive_path)

    def test_regular_files_remain_readable(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            archive_path = Path(temporary) / "artifact.tar.gz"
            payload = b"release"
            with tarfile.open(archive_path, "w:gz") as archive:
                member = tarfile.TarInfo("package/file.txt")
                member.size = len(payload)
                archive.addfile(member, io.BytesIO(payload))

            self.assertEqual(
                archive_entries(archive_path)[0].data,
                payload,
            )

    def test_mcp_bundle_requires_one_exact_prefix(self) -> None:
        entries = [
            Entry(f"cellrune-mcp-{EXPECTED_VERSION}-target/cellrune-mcp", b"binary"),
            Entry("other/LICENSE", b"license"),
            Entry(
                f"cellrune-mcp-{EXPECTED_VERSION}-target/THIRD_PARTY_LICENSES.md",
                b"Artifact target: `target`",
            ),
        ]
        with self.assertRaisesRegex(RuntimeError, r"^mcp\.boundary:"):
            validate_mcp(entries)

    def test_mcp_bundle_rejects_extra_files(self) -> None:
        prefix = f"cellrune-mcp-{EXPECTED_VERSION}-target"
        entries = [
            Entry(f"{prefix}/cellrune-mcp", b"binary"),
            Entry(f"{prefix}/LICENSE", b"license"),
            Entry(
                f"{prefix}/THIRD_PARTY_LICENSES.md",
                b"Artifact target: `target`",
            ),
            Entry(f"{prefix}/README.md", b"extra"),
        ]
        with self.assertRaisesRegex(RuntimeError, r"^mcp\.boundary:"):
            validate_mcp(entries)


class WheelPlatformTagTests(unittest.TestCase):
    """The platform tag is the whole of the wheel's compatibility promise."""

    @staticmethod
    def wheel_entries(tag: str) -> list[Entry]:
        return [
            Entry(
                "cellrune.dist-info/WHEEL",
                f"Wheel-Version: 1.0\nGenerator: maturin\nTag: {tag}\n".encode("utf-8"),
            )
        ]

    def test_declared_baseline_is_accepted(self) -> None:
        validate_wheel_platform_tag(self.wheel_entries("cp310-abi3-manylinux_2_28_x86_64"))

    def test_older_baseline_is_accepted(self) -> None:
        # A lower requirement installs everywhere the declared one does, and more besides.
        validate_wheel_platform_tag(self.wheel_entries("cp310-abi3-manylinux_2_17_x86_64"))

    def test_newer_baseline_is_rejected(self) -> None:
        with self.assertRaisesRegex(RuntimeError, r"^wheel\.linux_baseline:"):
            validate_wheel_platform_tag(self.wheel_entries("cp310-abi3-manylinux_2_35_x86_64"))

    def test_compressed_tag_set_cannot_hide_a_newer_baseline(self) -> None:
        # pip installs on a match against any component, so a legacy alias in first position makes
        # the wheel look installable on glibc 2.17 while the binary in it needs 2.35.
        with self.assertRaisesRegex(RuntimeError, r"^wheel\.linux_baseline:"):
            validate_wheel_platform_tag(
                self.wheel_entries("cp314-cp314-manylinux2014_x86_64.manylinux_2_35_x86_64")
            )

    def test_compressed_tag_set_within_the_baseline_is_accepted(self) -> None:
        validate_wheel_platform_tag(
            self.wheel_entries("cp314-cp314-manylinux2014_x86_64.manylinux_2_17_x86_64")
        )

    def test_unrepaired_linux_tag_is_rejected(self) -> None:
        with self.assertRaisesRegex(RuntimeError, r"^wheel\.linux_unrepaired:"):
            validate_wheel_platform_tag(self.wheel_entries("cp310-abi3-linux_x86_64"))

    def test_musl_and_non_linux_tags_carry_no_glibc_baseline(self) -> None:
        validate_wheel_platform_tag(self.wheel_entries("cp310-abi3-musllinux_1_2_x86_64"))
        validate_wheel_platform_tag(self.wheel_entries("cp310-abi3-macosx_11_0_arm64"))

    def test_missing_wheel_record_is_rejected(self) -> None:
        with self.assertRaisesRegex(RuntimeError, r"^wheel\.wheel_metadata:"):
            validate_wheel_platform_tag([Entry("cellrune/__init__.py", b"")])


if __name__ == "__main__":
    unittest.main()
