#!/usr/bin/env python3
"""Reject private paths, caches, and incomplete binding release archives."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import stat
import subprocess
import sys
import tarfile
import zipfile
from dataclasses import dataclass

ERROR_PREFIX = "binding artifact verification failed"
ERROR_MESSAGES = {
    "archive.empty": "archive is empty",
    "archive.member_unreadable": "could not read archive member {member}",
    "archive.special_entry": "links and special archive entries are forbidden: {name}",
    "archive.duplicate_member": "duplicate archive member path: {name}",
    "archive.type_unsupported": "unsupported archive type: {name}",
    "archive.path_unsafe": "unsafe archive member path: {name}",
    "archive.cache_directory": "cache or build directory included: {name}",
    "archive.cache_file": "generated cache file included: {name}",
    "archive.private_path": "private absolute path in {name}: {path}",
    "artifact.missing": "artifact does not exist: {path}",
    "artifact.none": "no artifacts were selected",
    "manifest.missing": "required manifest missing: {name}",
    "manifest.not_object": "manifest is not an object: {name}",
    "npm.version": "npm package version is not 0.1.0",
    "npm.homepage": "npm homepage does not match the public repository",
    "npm.repository": "npm repository metadata is missing or incorrect",
    "npm.issues": "npm issue tracker metadata is missing or incorrect",
    "npm.publish_config": "npm publishConfig is missing",
    "npm.public_provenance": "npm public access or provenance is not enabled",
    "npm.name": "npm package name is missing",
    "npm.root_boundary": "root npm archive boundary mismatch: {files}",
    "npm.root_native": "root npm archive must not contain a native binary",
    "npm.name_unexpected": "unexpected npm package name: {name}",
    "npm.platform_metadata": "platform npm archive is missing license or package metadata",
    "npm.platform_native": "platform npm archive must contain exactly one native binary",
    "npm.platform_boundary": "platform npm archive contains an unexpected file",
    "wheel.native": "wheel must contain exactly one native extension",
    "wheel.project_license": "wheel does not contain the CellRune license",
    "wheel.third_party": "wheel does not contain third-party license texts",
    "wheel.metadata": "wheel metadata is missing",
    "wheel.public_metadata": "wheel version or public project URLs are incomplete",
    "sdist.top_level": "sdist must have exactly one top-level directory",
    "sdist.required": "sdist is missing {name}",
    "sdist.compiled": "sdist contains a compiled artifact",
    "mcp.executable": "MCP bundle must contain exactly one server executable",
    "mcp.project_license": "MCP bundle does not contain the CellRune license",
    "mcp.third_party": "MCP bundle does not contain third-party license texts",
    "mcp.boundary": "MCP bundle contains unexpected files: {files}",
    "license.target_graph": "binary artifact does not contain a target-exact dependency notice",
    "license.union_graph": (
        "cross-platform source artifact does not contain the all-target dependency notice"
    ),
    "target.required": "wheel verification requires --expected-target",
    "target.host": "rustc did not report a host target",
}
EXPECTED_REPOSITORY = "git+https://github.com/emulette/cellrune.git"
EXPECTED_HOMEPAGE = "https://github.com/emulette/cellrune#readme"
EXPECTED_ISSUES = "https://github.com/emulette/cellrune/issues"
EXPECTED_VERSION = "0.1.0"
NOTICE_NAME = "THIRD_PARTY_LICENSES.md"
CACHE_SEGMENTS = frozenset(
    {
        ".git",
        ".mypy_cache",
        ".pytest_cache",
        ".venv",
        "__pycache__",
        "node_modules",
        "target",
    }
)
FORBIDDEN_SUFFIXES = (".pyc", ".pyo", ".DS_Store")
POSIX_USERS_ROOT = b"/" + b"Users" + b"/"
POSIX_HOME_ROOT = b"/" + b"home" + b"/"
PRIVATE_PATH_PATTERNS = (
    re.compile(rb"(?:file://)?" + re.escape(POSIX_USERS_ROOT) + rb"[A-Za-z0-9._-]+/"),
    re.compile(rb"(?:file://)?" + re.escape(POSIX_HOME_ROOT) + rb"[A-Za-z0-9._-]+/"),
    re.compile(rb"(?:file://)?/(?:private/)?(?:tmp|var/folders)/"),
    re.compile(rb"[A-Za-z]:[\\/]+Users[\\/]+[A-Za-z0-9._-]+[\\/]+", re.IGNORECASE),
)
ROOT_NPM_FILES = frozenset(
    {
        "package/LICENSE",
        "package/README.md",
        f"package/{NOTICE_NAME}",
        "package/index.d.ts",
        "package/index.mjs",
        "package/lib/changes.js",
        "package/lib/errors.js",
        "package/lib/normalization.js",
        "package/lib/validation.js",
        "package/native.d.ts",
        "package/native.js",
        "package/package.json",
        "package/wrapper.js",
    }
)
NPM_PLATFORM_TARGETS = {
    "@cellrune/node-darwin-arm64": "aarch64-apple-darwin",
    "@cellrune/node-darwin-x64": "x86_64-apple-darwin",
    "@cellrune/node-linux-arm64-gnu": "aarch64-unknown-linux-gnu",
    "@cellrune/node-linux-arm64-musl": "aarch64-unknown-linux-musl",
    "@cellrune/node-linux-x64-gnu": "x86_64-unknown-linux-gnu",
    "@cellrune/node-linux-x64-musl": "x86_64-unknown-linux-musl",
    "@cellrune/node-win32-arm64-msvc": "aarch64-pc-windows-msvc",
    "@cellrune/node-win32-x64-msvc": "x86_64-pc-windows-msvc",
}


@dataclass(frozen=True)
class Entry:
    name: str
    data: bytes


def artifact_error(code: str, **context: object) -> RuntimeError:
    template = ERROR_MESSAGES[code]
    return RuntimeError(f"{code}: {template.format(**context)}")


def archive_entries(path: pathlib.Path) -> list[Entry]:
    if path.suffix in {".whl", ".zip"}:
        with zipfile.ZipFile(path) as archive:
            entries: list[Entry] = []
            seen: set[str] = set()
            for info in archive.infolist():
                canonical_name = normalized_member_name(info.filename)
                if canonical_name in seen:
                    raise artifact_error(
                        "archive.duplicate_member", name=info.filename
                    )
                seen.add(canonical_name)
                if info.is_dir():
                    continue
                file_type = (info.external_attr >> 16) & 0o170000
                if file_type not in {0, stat.S_IFREG}:
                    raise artifact_error(
                        "archive.special_entry", name=info.filename
                    )
                entries.append(Entry(info.filename, archive.read(info)))
            return entries
    if path.name.endswith((".tgz", ".tar.gz")):
        with tarfile.open(path, mode="r:*") as archive:
            entries: list[Entry] = []
            seen = set()
            for member in archive.getmembers():
                canonical_name = normalized_member_name(member.name)
                if canonical_name in seen:
                    raise artifact_error(
                        "archive.duplicate_member", name=member.name
                    )
                seen.add(canonical_name)
                if member.isdir():
                    continue
                if not member.isfile():
                    raise artifact_error(
                        "archive.special_entry", name=member.name
                    )
                extracted = archive.extractfile(member)
                if extracted is None:
                    raise artifact_error(
                        "archive.member_unreadable", member=member.name
                    )
                entries.append(Entry(member.name, extracted.read()))
            return entries
    raise artifact_error("archive.type_unsupported", name=path.name)


def normalized_member_name(name: str) -> str:
    if not name or "\\" in name or re.match(r"^[A-Za-z]:", name):
        raise artifact_error("archive.path_unsafe", name=name)
    parts = name.rstrip("/").split("/")
    if not parts or any(part in {"", ".", ".."} for part in parts):
        raise artifact_error("archive.path_unsafe", name=name)
    member = pathlib.PurePosixPath(*parts)
    if member.is_absolute():
        raise artifact_error("archive.path_unsafe", name=name)
    if CACHE_SEGMENTS.intersection(member.parts):
        raise artifact_error("archive.cache_directory", name=name)
    if name.endswith(FORBIDDEN_SUFFIXES):
        raise artifact_error("archive.cache_file", name=name)
    return member.as_posix()


def validate_member_name(name: str) -> None:
    normalized_member_name(name)


def validate_payload_paths(entry: Entry) -> None:
    for pattern in PRIVATE_PATH_PATTERNS:
        match = pattern.search(entry.data)
        if match is not None:
            leaked = match.group(0).decode("utf-8", errors="replace")
            raise artifact_error(
                "archive.private_path", name=entry.name, path=leaked
            )


def parse_json_entry(entries: list[Entry], name: str) -> dict[str, object]:
    entry = next((candidate for candidate in entries if candidate.name == name), None)
    if entry is None:
        raise artifact_error("manifest.missing", name=name)
    raw: object = json.loads(entry.data)
    if not isinstance(raw, dict):
        raise artifact_error("manifest.not_object", name=name)
    return raw


def validate_target_notice(
    entries: list[Entry], expected_target: str | None = None
) -> None:
    notice = next(
        (
            entry.data
            for entry in entries
            if pathlib.PurePosixPath(entry.name).name == NOTICE_NAME
        ),
        None,
    )
    marker = (
        f"Artifact target: `{expected_target}`".encode("utf-8")
        if expected_target is not None
        else b"Artifact target: `"
    )
    if notice is None or marker not in notice:
        raise artifact_error("license.target_graph")


def validate_union_notice(entries: list[Entry]) -> None:
    notices = [
        entry.data
        for entry in entries
        if pathlib.PurePosixPath(entry.name).name == NOTICE_NAME
    ]
    if not notices or not any(b"conservative all-target union" in notice for notice in notices):
        raise artifact_error("license.union_graph")


def validate_npm_metadata(manifest: dict[str, object]) -> None:
    if manifest.get("version") != EXPECTED_VERSION:
        raise artifact_error("npm.version")
    if manifest.get("homepage") != EXPECTED_HOMEPAGE:
        raise artifact_error("npm.homepage")
    repository = manifest.get("repository")
    if not isinstance(repository, dict) or repository.get("url") != EXPECTED_REPOSITORY:
        raise artifact_error("npm.repository")
    bugs = manifest.get("bugs")
    if not isinstance(bugs, dict) or bugs.get("url") != EXPECTED_ISSUES:
        raise artifact_error("npm.issues")
    publish_config = manifest.get("publishConfig")
    if not isinstance(publish_config, dict):
        raise artifact_error("npm.publish_config")
    if publish_config.get("access") != "public" or publish_config.get("provenance") is not True:
        raise artifact_error("npm.public_provenance")


def validate_npm(entries: list[Entry]) -> str:
    manifest = parse_json_entry(entries, "package/package.json")
    validate_npm_metadata(manifest)
    package_name = manifest.get("name")
    if not isinstance(package_name, str):
        raise artifact_error("npm.name")
    names = frozenset(entry.name for entry in entries)
    native_files = sorted(name for name in names if name.endswith(".node"))
    if package_name == "@cellrune/node":
        if names != ROOT_NPM_FILES:
            unexpected = sorted(names.symmetric_difference(ROOT_NPM_FILES))
            raise artifact_error("npm.root_boundary", files=unexpected)
        if native_files:
            raise artifact_error("npm.root_native")
        validate_union_notice(entries)
        return "npm-root"

    if not package_name.startswith("@cellrune/node-"):
        raise artifact_error("npm.name_unexpected", name=package_name)
    required = {
        "package/LICENSE",
        "package/README.md",
        f"package/{NOTICE_NAME}",
        "package/package.json",
    }
    if not required.issubset(names):
        raise artifact_error("npm.platform_metadata")
    if len(native_files) != 1:
        raise artifact_error("npm.platform_native")
    if names != required | set(native_files):
        raise artifact_error("npm.platform_boundary")
    expected_target = NPM_PLATFORM_TARGETS.get(package_name)
    if expected_target is None:
        raise artifact_error("npm.name_unexpected", name=package_name)
    validate_target_notice(entries, expected_target)
    return "npm-platform"


def validate_wheel(entries: list[Entry], expected_target: str | None) -> str:
    names = [entry.name for entry in entries]
    native_files = [
        name for name in names if name.endswith((".so", ".pyd", ".dylib"))
    ]
    if len(native_files) != 1:
        raise artifact_error("wheel.native")
    if not any(name.endswith(".dist-info/licenses/LICENSE") for name in names):
        raise artifact_error("wheel.project_license")
    if not any(
        name.endswith(f".dist-info/licenses/{NOTICE_NAME}") for name in names
    ):
        raise artifact_error("wheel.third_party")
    metadata = next(
        (entry.data for entry in entries if entry.name.endswith(".dist-info/METADATA")),
        None,
    )
    if metadata is None:
        raise artifact_error("wheel.metadata")
    decoded = metadata.decode("utf-8")
    required_metadata = (
        "Version: 0.1.0",
        "Project-URL: Homepage, https://github.com/emulette/cellrune",
        "Project-URL: Issues, https://github.com/emulette/cellrune/issues",
    )
    if not all(value in decoded for value in required_metadata):
        raise artifact_error("wheel.public_metadata")
    if expected_target is None:
        raise artifact_error("target.required")
    validate_target_notice(entries, expected_target)
    return "wheel"


def validate_sdist(entries: list[Entry]) -> str:
    names = [entry.name for entry in entries]
    top_levels = {pathlib.PurePosixPath(name).parts[0] for name in names}
    if len(top_levels) != 1:
        raise artifact_error("sdist.top_level")
    prefix = next(iter(top_levels))
    required_suffixes = (
        "pyproject.toml",
        "bindings/common/Cargo.toml",
        "bindings/common/src/lib.rs",
        "bindings/python/Cargo.toml",
        f"bindings/python/{NOTICE_NAME}",
        "Cargo.lock",
    )
    for suffix in required_suffixes:
        if f"{prefix}/{suffix}" not in names:
            raise artifact_error("sdist.required", name=suffix)
    if any(name.endswith((".so", ".pyd", ".node", ".whl")) for name in names):
        raise artifact_error("sdist.compiled")
    validate_union_notice(entries)
    return "sdist"


def validate_mcp(entries: list[Entry]) -> str:
    names = [entry.name for entry in entries]
    basenames = [pathlib.PurePosixPath(name).name for name in names]
    binaries = [
        name
        for name in names
        if pathlib.PurePosixPath(name).name in {"cellrune-mcp", "cellrune-mcp.exe"}
    ]
    if len(binaries) != 1:
        raise artifact_error("mcp.executable")
    if "LICENSE" not in basenames:
        raise artifact_error("mcp.project_license")
    if NOTICE_NAME not in basenames:
        raise artifact_error("mcp.third_party")
    top_levels = {pathlib.PurePosixPath(name).parts[0] for name in names}
    if len(top_levels) != 1:
        raise artifact_error("mcp.boundary", files=sorted(names))
    prefix = next(iter(top_levels))
    binary_name = pathlib.PurePosixPath(binaries[0]).name
    expected_names = {
        f"{prefix}/{binary_name}",
        f"{prefix}/LICENSE",
        f"{prefix}/{NOTICE_NAME}",
    }
    if set(names) != expected_names:
        unexpected = sorted(set(names).symmetric_difference(expected_names))
        raise artifact_error("mcp.boundary", files=unexpected)
    expected_prefix = f"cellrune-mcp-{EXPECTED_VERSION}-"
    if not prefix.startswith(expected_prefix):
        raise artifact_error("license.target_graph")
    target = prefix[len(expected_prefix) :]
    if not target:
        raise artifact_error("license.target_graph")
    validate_target_notice(entries, target)
    return "mcp-binary"


def validate_archive(path: pathlib.Path, expected_target: str | None) -> str:
    entries = archive_entries(path)
    if not entries:
        raise artifact_error("archive.empty")
    for entry in entries:
        validate_member_name(entry.name)
        validate_payload_paths(entry)
    if path.suffix == ".whl":
        return validate_wheel(entries, expected_target)
    if path.name.endswith(".tgz"):
        return validate_npm(entries)
    if any(entry.name.endswith("/pyproject.toml") for entry in entries):
        return validate_sdist(entries)
    return validate_mcp(entries)


def validate_raw_binary(path: pathlib.Path) -> str:
    validate_payload_paths(Entry(path.name, path.read_bytes()))
    return "native-binary"


def resolve_expected_target(value: str | None) -> str | None:
    if value != "host":
        return value
    completed = subprocess.run(
        ["rustc", "-vV"],
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    match = re.search(r"^host: (?P<target>\S+)$", completed.stdout, re.MULTILINE)
    if match is None:
        raise artifact_error("target.host")
    return match.group("target")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("artifacts", nargs="+", type=pathlib.Path)
    parser.add_argument("--expected-target")
    arguments = parser.parse_args()
    expected_target = resolve_expected_target(arguments.expected_target)

    artifacts: list[pathlib.Path] = []
    for candidate in arguments.artifacts:
        if candidate.is_dir():
            artifacts.extend(sorted(path for path in candidate.iterdir() if path.is_file()))
        else:
            artifacts.append(candidate)
    if not artifacts:
        raise artifact_error("artifact.none")

    for artifact in artifacts:
        if not artifact.is_file():
            raise artifact_error("artifact.missing", path=artifact)
        kind = (
            validate_raw_binary(artifact)
            if artifact.name == "cellrune-mcp"
            or artifact.suffix in {".node", ".so", ".pyd", ".dylib", ".exe"}
            else validate_archive(artifact, expected_target)
        )
        digest = hashlib.sha256(artifact.read_bytes()).hexdigest()
        print(f"{digest}  {artifact}  ({kind})")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (
        OSError,
        RuntimeError,
        subprocess.CalledProcessError,
        tarfile.TarError,
        zipfile.BadZipFile,
    ) as error:
        print(f"{ERROR_PREFIX}: {error}", file=sys.stderr)
        raise SystemExit(1) from error
