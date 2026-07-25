#!/usr/bin/env python3
"""Read the release version from the one manifest that owns it.

Every artifact CellRune publishes carries the same version, and the release gates compare artifact
names and embedded metadata against it. Repeating the literal in each script means a release can
fail after the version has been bumped everywhere a human remembered to look, which is the failure
this module exists to prevent. The workspace manifest is the single source, and every other
manifest is checked against it by ``verify_release_artifacts.py``.
"""

from __future__ import annotations

import pathlib
import re

ERROR_PREFIX = "workspace version lookup failed"
MESSAGE_MANIFEST_MISSING = "workspace manifest not found at {path}"
MESSAGE_VERSION_MISSING = "no [workspace.package] version in {path}"

_WORKSPACE_PACKAGE = re.compile(
    r"^\[workspace\.package\]\s*$(.*?)(?=^\[|\Z)",
    re.MULTILINE | re.DOTALL,
)
_VERSION = re.compile(r'^version\s*=\s*"([^"]+)"\s*$', re.MULTILINE)


def repository_root() -> pathlib.Path:
    """Return the repository root, resolved from this file rather than the caller's cwd."""
    return pathlib.Path(__file__).resolve().parents[2]


def workspace_version(root: pathlib.Path | None = None) -> str:
    """Return the version declared by ``[workspace.package]`` in the workspace manifest."""
    manifest = (root or repository_root()) / "Cargo.toml"
    if not manifest.is_file():
        raise RuntimeError(f"{ERROR_PREFIX}: {MESSAGE_MANIFEST_MISSING.format(path=manifest)}")
    section = _WORKSPACE_PACKAGE.search(manifest.read_text(encoding="utf-8"))
    version = _VERSION.search(section.group(1)) if section else None
    if version is None:
        raise RuntimeError(f"{ERROR_PREFIX}: {MESSAGE_VERSION_MISSING.format(path=manifest)}")
    return version.group(1)


if __name__ == "__main__":
    print(workspace_version())
