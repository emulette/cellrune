#!/usr/bin/env python3
"""Extract and exercise the executable from a verified MCP release bundle."""

from __future__ import annotations

import argparse
import pathlib
import subprocess
import tempfile

from verify_release_artifacts import archive_entries, validate_member_name

ERROR_PREFIX = "MCP bundle consumer test failed"
MESSAGE_ARCHIVE_COUNT = "expected exactly one MCP archive under {path}"
MESSAGE_EXECUTABLE_COUNT = "expected exactly one MCP executable in the bundle"
MESSAGE_HELP_CONTRACT = "bundled MCP executable did not expose the expected help contract"


def resolve_archive(path: pathlib.Path) -> pathlib.Path:
    if path.is_file():
        return path
    archives = sorted(
        candidate
        for candidate in path.iterdir()
        if candidate.is_file()
        and (
            candidate.suffix == ".zip"
            or candidate.name.endswith(".tar.gz")
        )
    )
    if len(archives) != 1:
        raise RuntimeError(MESSAGE_ARCHIVE_COUNT.format(path=path))
    return archives[0]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("bundle", type=pathlib.Path)
    arguments = parser.parse_args()
    archive = resolve_archive(arguments.bundle)

    with tempfile.TemporaryDirectory(prefix="cellrune-mcp-consumer-") as temporary:
        root = pathlib.Path(temporary)
        executables: list[pathlib.Path] = []
        for entry in archive_entries(archive):
            validate_member_name(entry.name)
            destination = root.joinpath(*pathlib.PurePosixPath(entry.name).parts)
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(entry.data)
            if destination.name in {"cellrune-mcp", "cellrune-mcp.exe"}:
                destination.chmod(0o755)
                executables.append(destination)
        if len(executables) != 1:
            raise RuntimeError(MESSAGE_EXECUTABLE_COUNT)

        completed = subprocess.run(
            [str(executables[0]), "--help"],
            cwd=root,
            check=True,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        if "Usage: cellrune-mcp" not in completed.stdout:
            raise RuntimeError(MESSAGE_HELP_CONTRACT)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        raise SystemExit(f"{ERROR_PREFIX}: {error}") from error
