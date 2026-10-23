#!/usr/bin/env python3
"""Check the MCP Bundle and registry entry ``mcp_registry.py`` builds from release archives."""

from __future__ import annotations

import hashlib
import json
import os
import pathlib
import re
import tempfile
import unittest
import zipfile

import mcp_registry
from package_mcp import LICENSE_NAMES, NOTICE_NAME, write_tar, write_zip
from workspace_version import workspace_version

VERSION = workspace_version()


def write_release_archives(directory: pathlib.Path, license_text: bytes = b"license") -> None:
    """Write one archive per target as ``package_mcp.py`` lays it out, with stand-in files."""
    sources = directory / "sources"
    sources.mkdir()
    for target, platform, _ in mcp_registry.TARGETS:
        files = []
        for name, content, mode in (
            (mcp_registry.executable_name(platform), f"binary {target}".encode(), 0o755),
            (NOTICE_NAME, f"notices {target}".encode(), 0o644),
            *((name, license_text, 0o644) for name in LICENSE_NAMES),
        ):
            source = sources / f"{target}-{name}"
            source.write_bytes(content)
            files.append((name, source, mode))
        prefix = f"cellrune-mcp-{VERSION}-{target}"
        if platform == "win32":
            write_zip(directory / f"{prefix}.zip", prefix, tuple(files), 0)
        else:
            write_tar(directory / f"{prefix}.tar.gz", prefix, tuple(files), 0)


class McpRegistryTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="cellrune-mcp-registry-")
        self.root = pathlib.Path(self.temporary.name)
        self.archives = self.root / "archives"
        self.archives.mkdir()
        os.environ["SOURCE_DATE_EPOCH"] = "1700000000"

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def test_bundle_selects_each_platform_executable(self) -> None:
        write_release_archives(self.archives)
        bundle = mcp_registry.bundle(self.archives, self.root / "out")
        self.assertEqual(bundle.name, f"cellrune-mcp-{VERSION}.mcpb")
        with zipfile.ZipFile(bundle) as archive:
            names = set(archive.namelist())
            manifest = json.loads(archive.read("manifest.json"))
            for target, platform, directory in mcp_registry.TARGETS:
                executable = f"server/{directory}/{mcp_registry.executable_name(platform)}"
                self.assertEqual(archive.read(executable), f"binary {target}".encode())
                mode = (archive.getinfo(executable).external_attr >> 16) & 0o777
                self.assertEqual(mode, 0o755)
                self.assertEqual(
                    archive.read(f"server/{directory}/{NOTICE_NAME}"),
                    f"notices {target}".encode(),
                )
        self.assertEqual(
            names,
            {
                "manifest.json",
                *LICENSE_NAMES,
                *(
                    f"server/{directory}/{name}"
                    for _, platform, directory in mcp_registry.TARGETS
                    for name in (mcp_registry.executable_name(platform), NOTICE_NAME)
                ),
            },
        )
        self.assertEqual(manifest["version"], VERSION)
        self.assertEqual(manifest["server"]["type"], "binary")
        self.assertIn(manifest["server"]["entry_point"], names)
        config = manifest["server"]["mcp_config"]
        self.assertEqual(config["args"], ["--root", "${user_config.root}"])
        commands = [config["command"], *(o["command"] for o in config["platform_overrides"].values())]
        for command in commands:
            self.assertIn(command.removeprefix("${__dirname}/"), names)
        self.assertEqual(
            set(config["platform_overrides"]) | {"linux"},
            set(manifest["compatibility"]["platforms"]),
        )
        self.assertTrue(manifest["user_config"]["root"]["required"])

        # The same inputs and timestamp produce the same bytes, so a rebuilt bundle keeps its digest.
        again = mcp_registry.bundle(self.archives, self.root / "again")
        self.assertEqual(bundle.read_bytes(), again.read_bytes())

    def test_server_json_names_the_exact_bundle(self) -> None:
        write_release_archives(self.archives)
        bundle = mcp_registry.bundle(self.archives, self.root / "out")
        entry = mcp_registry.server_json(bundle)
        for required in ("name", "description", "version"):
            self.assertIn(required, entry)
        self.assertRegex(entry["name"], r"^[a-zA-Z0-9.-]+/[a-zA-Z0-9._-]+$")
        self.assertLessEqual(len(entry["description"]), 100)
        self.assertLessEqual(len(entry["title"]), 100)
        self.assertEqual(entry["version"], VERSION)
        (package,) = entry["packages"]
        self.assertEqual(package["registryType"], "mcpb")
        self.assertEqual(package["transport"], {"type": "stdio"})
        self.assertEqual(
            package["identifier"],
            f"https://github.com/emulette/cellrune/releases/download/v{VERSION}/{bundle.name}",
        )
        self.assertIn("mcp", package["identifier"].rsplit("/", 1)[1])
        self.assertTrue(re.fullmatch(r"[a-f0-9]{64}", package["fileSha256"]))
        self.assertEqual(package["fileSha256"], hashlib.sha256(bundle.read_bytes()).hexdigest())

    def test_missing_or_inconsistent_archives_are_rejected(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "no release archive"):
            mcp_registry.bundle(self.archives, self.root / "out")
        write_release_archives(self.archives)
        windows = self.archives / f"cellrune-mcp-{VERSION}-x86_64-pc-windows-msvc.zip"
        with zipfile.ZipFile(windows, "a") as archive:
            prefix = f"cellrune-mcp-{VERSION}-x86_64-pc-windows-msvc"
            archive.writestr(f"{prefix}/{LICENSE_NAMES[0]}.extra", b"")
        mcp_registry.bundle(self.archives, self.root / "out")

        mismatched = self.root / "mismatched"
        mismatched.mkdir()
        write_release_archives(mismatched)
        darwin = mismatched / f"cellrune-mcp-{VERSION}-aarch64-apple-darwin.tar.gz"
        darwin.unlink()
        sources = self.root / "darwin-sources"
        sources.mkdir()
        files = []
        for name, content, mode in (
            ("cellrune-mcp", b"binary", 0o755),
            (NOTICE_NAME, b"notices", 0o644),
            *((name, b"other license", 0o644) for name in LICENSE_NAMES),
        ):
            source = sources / name
            source.write_bytes(content)
            files.append((name, source, mode))
        write_tar(darwin, f"cellrune-mcp-{VERSION}-aarch64-apple-darwin", tuple(files), 0)
        with self.assertRaisesRegex(RuntimeError, "differs between the release archives"):
            mcp_registry.bundle(mismatched, self.root / "mismatched-out")


if __name__ == "__main__":
    unittest.main()
