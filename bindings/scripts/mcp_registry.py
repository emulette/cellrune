#!/usr/bin/env python3
"""Assemble the MCP Bundle and the MCP registry ``server.json`` of a CellRune release.

The official MCP registry lists a native stdio server as an MCP Bundle (``.mcpb``) attached to a
GitHub release, identified by its download URL and the SHA-256 of that exact file. The release
workflow attaches one verified archive per platform; ``bundle`` combines the three into one bundle
whose manifest selects the platform's executable, and ``server-json`` writes the registry metadata
for the bundle it is given, so the version and digest always describe the published file.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import stat
import tarfile
import time
import zipfile

from package_mcp import LICENSE_NAMES, NOTICE_NAME, source_date_epoch
from verify_release_artifacts import archive_entries, validate_member_name
from workspace_version import workspace_version

ERROR_PREFIX = "MCP registry preparation failed"
MESSAGE_ARCHIVE_MISSING = "no release archive for {target} under {path}"
MESSAGE_ARCHIVE_DUPLICATE = "more than one release archive for {target} under {path}"
MESSAGE_MEMBER_MISSING = "{archive} does not contain {name}"
MESSAGE_LICENSE_MISMATCH = "{name} differs between the release archives"

REGISTRY_NAME = "io.github.emulette/cellrune"
REPOSITORY_URL = "https://github.com/emulette/cellrune"
SERVER_SCHEMA = "https://static.modelcontextprotocol.io/schemas/2025-12-11/server.schema.json"
DESCRIPTION = "Read, calculate, edit, and save XLSX workbooks headlessly over local stdio."
TITLE = "CellRune"
MANIFEST_VERSION = "0.3"
EXECUTABLE_MODE = 0o755
FILE_MODE = 0o644

# Release target, MCPB platform, and bundle directory of each executable the release builds.
TARGETS: tuple[tuple[str, str, str], ...] = (
    ("x86_64-unknown-linux-gnu", "linux", "linux-x64"),
    ("aarch64-apple-darwin", "darwin", "darwin-arm64"),
    ("x86_64-pc-windows-msvc", "win32", "win32-x64"),
)


def executable_name(platform: str) -> str:
    return "cellrune-mcp.exe" if platform == "win32" else "cellrune-mcp"


def release_archive(directory: pathlib.Path, version: str, target: str) -> pathlib.Path:
    stem = f"cellrune-mcp-{version}-{target}"
    matches = [
        candidate
        for candidate in (directory / f"{stem}.tar.gz", directory / f"{stem}.zip")
        if candidate.is_file()
    ]
    if not matches:
        raise RuntimeError(MESSAGE_ARCHIVE_MISSING.format(target=target, path=directory))
    if len(matches) > 1:
        raise RuntimeError(MESSAGE_ARCHIVE_DUPLICATE.format(target=target, path=directory))
    return matches[0]


def archive_members(archive: pathlib.Path, version: str, target: str) -> dict[str, bytes]:
    """Return the archive's files by their name below the archive's single top directory."""
    prefix = f"cellrune-mcp-{version}-{target}/"
    members: dict[str, bytes] = {}
    for entry in archive_entries(archive):
        validate_member_name(entry.name)
        if entry.name.startswith(prefix):
            members[entry.name[len(prefix) :]] = entry.data
    return members


def manifest(version: str) -> dict[str, object]:
    """Return the MCPB manifest that runs the platform's executable on one approved root."""
    commands = {
        platform: f"${{__dirname}}/server/{directory}/{executable_name(platform)}"
        for _, platform, directory in TARGETS
    }
    default_platform = TARGETS[0][1]
    return {
        "manifest_version": MANIFEST_VERSION,
        "name": "cellrune",
        "display_name": TITLE,
        "version": version,
        "description": DESCRIPTION,
        "author": {"name": "CellRune contributors"},
        "license": "MIT OR Apache-2.0",
        "repository": {"type": "git", "url": REPOSITORY_URL},
        "homepage": REPOSITORY_URL,
        "server": {
            "type": "binary",
            "entry_point": f"server/{TARGETS[0][2]}/{executable_name(default_platform)}",
            "mcp_config": {
                "command": commands[default_platform],
                "args": ["--root", "${user_config.root}"],
                "env": {},
                "platform_overrides": {
                    platform: {"command": command}
                    for platform, command in commands.items()
                    if platform != default_platform
                },
            },
        },
        "compatibility": {"platforms": sorted(commands)},
        "user_config": {
            "root": {
                "type": "directory",
                "title": "Workbook directory",
                "description": "The only directory whose workbooks the server may open and save.",
                "required": True,
            }
        },
    }


def bundle(archives: pathlib.Path, output: pathlib.Path) -> pathlib.Path:
    """Write ``cellrune-mcp-<version>.mcpb`` from the three verified release archives."""
    version = workspace_version()
    files: dict[str, tuple[bytes, int]] = {}
    for target, platform, directory in TARGETS:
        archive = release_archive(archives, version, target)
        members = archive_members(archive, version, target)
        for name in (executable_name(platform), NOTICE_NAME, *LICENSE_NAMES):
            if name not in members:
                raise RuntimeError(MESSAGE_MEMBER_MISSING.format(archive=archive, name=name))
        files[f"server/{directory}/{executable_name(platform)}"] = (
            members[executable_name(platform)],
            EXECUTABLE_MODE,
        )
        files[f"server/{directory}/{NOTICE_NAME}"] = (members[NOTICE_NAME], FILE_MODE)
        for name in LICENSE_NAMES:
            previous = files.setdefault(name, (members[name], FILE_MODE))
            if previous[0] != members[name]:
                raise RuntimeError(MESSAGE_LICENSE_MISMATCH.format(name=name))
    files["manifest.json"] = (
        (json.dumps(manifest(version), indent=2) + "\n").encode("utf-8"),
        FILE_MODE,
    )
    output.mkdir(parents=True, exist_ok=True)
    destination = output / f"cellrune-mcp-{version}.mcpb"
    timestamp = time.gmtime(max(source_date_epoch(), 315532800))[:6]
    with zipfile.ZipFile(
        destination, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9
    ) as archive:
        for name in sorted(files):
            data, mode = files[name]
            info = zipfile.ZipInfo(name, date_time=timestamp)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = (stat.S_IFREG | mode) << 16
            archive.writestr(info, data)
    return destination


def server_json(bundle_path: pathlib.Path) -> dict[str, object]:
    """Return the registry entry for the bundle published with the release tag."""
    version = workspace_version()
    return {
        "$schema": SERVER_SCHEMA,
        "name": REGISTRY_NAME,
        "title": TITLE,
        "description": DESCRIPTION,
        "version": version,
        "websiteUrl": REPOSITORY_URL,
        "repository": {
            "url": REPOSITORY_URL,
            "source": "github",
            "subfolder": "crates/cellrune-mcp",
        },
        "packages": [
            {
                "registryType": "mcpb",
                "identifier": (
                    f"{REPOSITORY_URL}/releases/download/v{version}/{bundle_path.name}"
                ),
                "version": version,
                "fileSha256": hashlib.sha256(bundle_path.read_bytes()).hexdigest(),
                "transport": {"type": "stdio"},
            }
        ],
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    bundle_command = commands.add_parser("bundle", help="assemble the MCP Bundle")
    bundle_command.add_argument("--archives", required=True, type=pathlib.Path)
    bundle_command.add_argument("--output", required=True, type=pathlib.Path)
    server_command = commands.add_parser("server-json", help="write the registry entry")
    server_command.add_argument("--bundle", required=True, type=pathlib.Path)
    server_command.add_argument("--output", required=True, type=pathlib.Path)
    arguments = parser.parse_args()
    if arguments.command == "bundle":
        print(bundle(arguments.archives, arguments.output))
    else:
        entry = server_json(arguments.bundle)
        arguments.output.write_text(json.dumps(entry, indent=2) + "\n", encoding="utf-8")
        print(arguments.output)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, tarfile.TarError, zipfile.BadZipFile) as error:
        raise SystemExit(f"{ERROR_PREFIX}: {error}") from error
