#!/usr/bin/env python3
"""Assert every published version declaration agrees with the workspace manifest.

CellRune publishes one version across three registries and a GitHub Release, and several gates
resolve with `--locked` or `--frozen-lockfile`. A bump that reaches most declarations but not all
of them does not fail where it was made; it fails much later, inside a release run, after the
irreversible steps have already been approved. The 0.1.1 bump found stale declarations in eight
places beyond the documented checklist, three of which would have failed a release outright.

This gate derives the expected version from `[workspace.package]` and checks every other
declaration against it, so an incomplete bump fails on the pull request that made it.
"""

from __future__ import annotations

import json
import pathlib
import re
import sys

from workspace_version import repository_root, workspace_version

ERROR_PREFIX = "version consistency check failed"
MESSAGE_MISMATCH = "{path}: {label} is {actual}, expected {expected}"
MESSAGE_MISSING = "{path}: no {label} found"
MESSAGE_UNREADABLE = "{path}: cannot be read"

NODE_PLATFORM_DIRECTORY = "bindings/node/npm"


def _fail(messages: list[str]) -> int:
    for message in messages:
        print(f"{ERROR_PREFIX}: {message}", file=sys.stderr)
    return 1


def _json_version(root: pathlib.Path, relative: str) -> tuple[str, str | None]:
    path = root / relative
    if not path.is_file():
        return relative, None
    return relative, json.loads(path.read_text(encoding="utf-8")).get("version")


def _text_matches(root: pathlib.Path, relative: str, pattern: re.Pattern[str]) -> list[str]:
    path = root / relative
    if not path.is_file():
        return []
    return pattern.findall(path.read_text(encoding="utf-8"))


def check(root: pathlib.Path, expected: str) -> list[str]:
    problems: list[str] = []

    # Python distribution metadata.
    pyproject = _text_matches(
        root,
        "bindings/python/pyproject.toml",
        re.compile(r'^version\s*=\s*"([^"]+)"', re.MULTILINE),
    )
    if not pyproject:
        problems.append(
            MESSAGE_MISSING.format(path="bindings/python/pyproject.toml", label="version")
        )
    elif pyproject[0] != expected:
        problems.append(
            MESSAGE_MISMATCH.format(
                path="bindings/python/pyproject.toml",
                label="version",
                actual=pyproject[0],
                expected=expected,
            )
        )

    # Root npm manifest, its optional platform dependencies, and each platform manifest.
    relative, version = _json_version(root, "bindings/node/package.json")
    if version is None:
        problems.append(MESSAGE_MISSING.format(path=relative, label="version"))
    elif version != expected:
        problems.append(
            MESSAGE_MISMATCH.format(
                path=relative, label="version", actual=version, expected=expected
            )
        )

    manifest_path = root / "bindings/node/package.json"
    if manifest_path.is_file():
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        for name, requirement in sorted(
            manifest.get("optionalDependencies", {}).items()
        ):
            if requirement != expected:
                problems.append(
                    MESSAGE_MISMATCH.format(
                        path="bindings/node/package.json",
                        label=f"optionalDependencies['{name}']",
                        actual=requirement,
                        expected=expected,
                    )
                )

    platform_root = root / NODE_PLATFORM_DIRECTORY
    for platform_manifest in sorted(platform_root.glob("*/package.json")):
        relative = str(platform_manifest.relative_to(root))
        version = json.loads(platform_manifest.read_text(encoding="utf-8")).get("version")
        if version != expected:
            problems.append(
                MESSAGE_MISMATCH.format(
                    path=relative, label="version", actual=version, expected=expected
                )
            )

    # pnpm resolves platform packages with `--frozen-lockfile`, so a stale specifier is fatal.
    # Only the workspace's own platform packages track the release version; third-party
    # development dependencies legitimately carry their own.
    lock_path = root / "bindings/node/pnpm-lock.yaml"
    if lock_path.is_file():
        lock_text = lock_path.read_text(encoding="utf-8")
        for match in re.finditer(
            r"'(@cellrune/node-[a-z0-9-]+)':\s*\n\s+specifier:\s*([^\s]+)", lock_text
        ):
            if match.group(2) != expected:
                problems.append(
                    MESSAGE_MISMATCH.format(
                        path="bindings/node/pnpm-lock.yaml",
                        label=f"specifier for {match.group(1)}",
                        actual=match.group(2),
                        expected=expected,
                    )
                )

    # Intra-workspace requirements release in lockstep and are pinned exactly.
    for manifest in sorted(root.glob("**/Cargo.toml")):
        if any(
            segment in {"target", "node_modules", ".git"} for segment in manifest.parts
        ):
            continue
        relative = str(manifest.relative_to(root))
        for match in re.finditer(
            r'(cellrune[a-z-]*)\s*=\s*\{[^}]*version\s*=\s*"=([^"]+)"', manifest.read_text(
                encoding="utf-8"
            )
        ):
            if match.group(2) != expected:
                problems.append(
                    MESSAGE_MISMATCH.format(
                        path=relative,
                        label=f"exact requirement on {match.group(1)}",
                        actual=match.group(2),
                        expected=expected,
                    )
                )

    # The generated napi shim embeds the version it enforces at load time.
    native = _text_matches(
        root,
        "bindings/node/native.js",
        re.compile(r"bindingPackageVersion !== '(\d+\.\d+\.\d+)'"),
    )
    for found in set(native):
        if found != expected:
            problems.append(
                MESSAGE_MISMATCH.format(
                    path="bindings/node/native.js",
                    label="enforced binding version",
                    actual=found,
                    expected=expected,
                )
            )

    # The changelog drives the release notes, so the version must have its own dated section.
    changelog = root / "CHANGELOG.md"
    if changelog.is_file():
        if not re.search(
            rf"^## \[{re.escape(expected)}\] - \d{{4}}-\d{{2}}-\d{{2}}$",
            changelog.read_text(encoding="utf-8"),
            re.MULTILINE,
        ):
            problems.append(
                MESSAGE_MISSING.format(path="CHANGELOG.md", label=f"dated [{expected}] section")
            )

    return problems


def main() -> int:
    root = repository_root()
    expected = workspace_version(root)
    problems = check(root, expected)
    if problems:
        return _fail(problems)
    print(f"every published version declaration agrees with the workspace version {expected}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
