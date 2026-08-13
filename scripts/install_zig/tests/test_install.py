from __future__ import annotations

import os
import io
import pathlib
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest import mock


MODULE_DIRECTORY = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(MODULE_DIRECTORY))

import errors  # noqa: E402
import install  # noqa: E402


class ZigInstallerTests(unittest.TestCase):
    def test_archive_members_must_remain_relative_to_the_extract_root(self) -> None:
        self.assertTrue(install.is_safe_member("zig-linux/zig"))
        self.assertFalse(install.is_safe_member("/tmp/zig"))
        self.assertFalse(install.is_safe_member("zig-linux/../../tmp/zig"))
        self.assertFalse(install.is_safe_member(r"zig-linux\..\tmp\zig"))

    def test_manual_extraction_preserves_regular_file_mode(self) -> None:
        payload = b"#!/bin/sh\n"
        with tempfile.TemporaryDirectory() as directory:
            archive_path = pathlib.Path(directory) / "zig.tar"
            with tarfile.open(archive_path, "w") as archive:
                member = tarfile.TarInfo("zig-linux/zig")
                member.size = len(payload)
                member.mode = 0o755
                archive.addfile(member, io.BytesIO(payload))
            destination = pathlib.Path(directory) / "extract"
            destination.mkdir()
            with tarfile.open(archive_path, "r") as archive:
                for member in archive.getmembers():
                    install.extract_member(archive, member, destination)
            binary = destination / "zig-linux" / "zig"
            self.assertEqual(binary.read_bytes(), payload)
            self.assertEqual(binary.stat().st_mode & 0o777, 0o755)

    def test_manual_extraction_rejects_links_and_devices(self) -> None:
        for member_type in (tarfile.SYMTYPE, tarfile.LNKTYPE, tarfile.CHRTYPE):
            with self.subTest(member_type=member_type):
                member = tarfile.TarInfo("zig-linux/member")
                member.type = member_type
                with tempfile.TemporaryDirectory() as directory:
                    archive_path = pathlib.Path(directory) / "zig.tar"
                    with tarfile.open(archive_path, "w") as archive:
                        archive.addfile(member)
                    destination = pathlib.Path(directory) / "extract"
                    destination.mkdir()
                    with tarfile.open(archive_path, "r") as archive:
                        loaded = archive.getmembers()[0]
                        with self.assertRaises(errors.InstallerError) as caught:
                            install.extract_member(archive, loaded, destination)
                self.assertEqual(caught.exception.code, errors.UNSAFE_ARCHIVE_MEMBER)

    def test_install_root_requires_a_runner_owned_directory(self) -> None:
        with mock.patch.dict(os.environ, {}, clear=True):
            with self.assertRaises(errors.InstallerError) as caught:
                install.install_root_path()
        self.assertEqual(caught.exception.code, errors.UNSAFE_INSTALL_ROOT)

    def test_runner_temp_is_the_preferred_install_root(self) -> None:
        with tempfile.TemporaryDirectory() as runner_temp:
            with tempfile.TemporaryDirectory() as tool_cache:
                with mock.patch.dict(
                    os.environ,
                    {"RUNNER_TEMP": runner_temp, "RUNNER_TOOL_CACHE": tool_cache},
                    clear=True,
                ):
                    self.assertEqual(install.install_root_path(), pathlib.Path(runner_temp))

    def test_publish_toolchain_appends_the_directory_to_github_path(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            github_path = pathlib.Path(directory) / "github-path"
            toolchain = pathlib.Path(directory) / "zig"
            with mock.patch.dict(os.environ, {"GITHUB_PATH": str(github_path)}, clear=True):
                install.publish_toolchain(toolchain)
            self.assertEqual(github_path.read_text(encoding="utf-8"), f"{toolchain}\n")

    def test_verify_toolchain_rejects_an_unexpected_version(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            binary = pathlib.Path(directory) / "zig"
            binary.touch()
            result = subprocess.CompletedProcess([str(binary), "version"], 0, "0.14.0\n", "")
            with mock.patch.object(install.subprocess, "run", return_value=result):
                with self.assertRaises(errors.InstallerError) as caught:
                    install.verify_toolchain(pathlib.Path(directory))
        self.assertEqual(caught.exception.code, errors.VERSION_MISMATCH)


if __name__ == "__main__":
    unittest.main()
