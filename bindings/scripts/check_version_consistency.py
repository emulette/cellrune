#!/usr/bin/env python3
"""Assert every published version declaration agrees with the workspace manifest.

CellRune publishes one version across three registries and a GitHub Release, and several build
steps resolve with `--locked` or `--frozen-lockfile`. A bump that reaches most declarations but not
all of them otherwise fails much later in the release. The 0.1.1 bump found stale declarations in
eight places beyond the documented checklist, three of which would have failed a release outright.

This release-contract check derives the expected version from `[workspace.package]` and checks
every other declaration against it. Developers run it locally while preparing a release, and the
tag workflow runs it before artifact builds or any publication job.

Every check here is written so that finding nothing is a failure rather than a pass. A gate whose
regex stops matching because a generator changed its output, or whose file was moved, would
otherwise report the same green result as a correct bump — and would do so precisely when a
declaration had drifted out from under it.
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
MESSAGE_UNREADABLE = "{path}: cannot be read: {reason}"

NODE_PLATFORM_DIRECTORY = "bindings/node/npm"
NODE_PLATFORM_SCOPE = "@cellrune/node-"
SKIPPED_DIRECTORIES = frozenset({"target", "node_modules", ".git"})

# Documentation and policy files that name the release version in prose or in an install command.
# A stale literal here is not caught by any build: it ships on the crates.io and PyPI project
# pages and on the repository front page, telling users to install a version that is not current.
PROSE_VERSION_PATTERNS: tuple[tuple[str, str, str], ...] = (
    ("README.md", r"The CellRune Rust crate ([0-9][^\s]*) requires", "crate version"),
    ("README.md", r"cargo add cellrune@([^\s`]+)", "cargo add version"),
    ("README.md", r'^cellrune = "([^"]+)"', "crate requirement"),
    ("README.md", r"The ([0-9][^\s]*) release line targets", "release line"),
    ("README.md", r'pip install "cellrune==([^"]+)"', "pip install version"),
    ("README.md", r'npm install "@cellrune/node@([^"]+)"', "npm install version"),
    ("bindings/node/README.md", r"npm install @cellrune/node@([^\s`]+)", "npm install version"),
    ("llms.txt", r"The current public version is ([0-9][^\s,]*)", "public version"),
    (
        "THIRD_PARTY_LICENSES.md",
        r"for CellRune (\d+\.\d+\.\d+)\.",
        "runtime license graph version",
    ),
    (
        "crates/cellrune/THIRD_PARTY_LICENSES.md",
        r"for CellRune (\d+\.\d+\.\d+)\.",
        "packaged runtime license graph version",
    ),
)


def _fail(messages: list[str]) -> int:
    for message in messages:
        print(f"{ERROR_PREFIX}: {message}", file=sys.stderr)
    return 1


def _read_json(path: pathlib.Path, relative: str, problems: list[str]) -> dict | None:
    """Parse a JSON manifest, recording a diagnostic instead of raising on malformed input."""
    if not path.is_file():
        problems.append(MESSAGE_MISSING.format(path=relative, label="file"))
        return None
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        problems.append(MESSAGE_UNREADABLE.format(path=relative, reason=error))
        return None


def _read_text(path: pathlib.Path, relative: str, problems: list[str]) -> str | None:
    if not path.is_file():
        problems.append(MESSAGE_MISSING.format(path=relative, label="file"))
        return None
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as error:
        problems.append(MESSAGE_UNREADABLE.format(path=relative, reason=error))
        return None


def _check_matches(
    text: str,
    relative: str,
    pattern: re.Pattern[str],
    label: str,
    expected: str,
    problems: list[str],
    *,
    minimum: int = 1,
) -> None:
    """Compare every capture of ``pattern`` against ``expected``.

    ``minimum`` is the number of matches the declaration is known to contain. Falling below it
    means the pattern no longer describes the file, which has to be reported: a check that matches
    nothing agrees with everything.
    """
    found = pattern.findall(text)
    if len(found) < minimum:
        problems.append(
            MESSAGE_MISSING.format(path=relative, label=f"{label} (expected {minimum} or more)")
        )
        return
    for actual in sorted(set(found)):
        if actual != expected:
            problems.append(
                MESSAGE_MISMATCH.format(
                    path=relative, label=label, actual=actual, expected=expected
                )
            )


def check(root: pathlib.Path, expected: str) -> list[str]:
    problems: list[str] = []

    # Python distribution metadata.
    relative = "bindings/python/pyproject.toml"
    if (text := _read_text(root / relative, relative, problems)) is not None:
        _check_matches(
            text,
            relative,
            re.compile(r'^version\s*=\s*"([^"]+)"', re.MULTILINE),
            "version",
            expected,
            problems,
        )

    # Root npm manifest, its optional platform dependencies, and each platform manifest.
    relative = "bindings/node/package.json"
    manifest = _read_json(root / relative, relative, problems)
    if manifest is not None:
        version = manifest.get("version")
        if version is None:
            problems.append(MESSAGE_MISSING.format(path=relative, label="version"))
        elif version != expected:
            problems.append(
                MESSAGE_MISMATCH.format(
                    path=relative, label="version", actual=version, expected=expected
                )
            )

        # Only the workspace's own platform packages track the release version. A third-party
        # optional dependency legitimately carries its own, so requiring every entry to equal the
        # release version would reject any ordinary third-party optional dependency.
        optional = manifest.get("optionalDependencies", {})
        platform_requirements = {
            name: requirement
            for name, requirement in optional.items()
            if name.startswith(NODE_PLATFORM_SCOPE)
        }
        if not platform_requirements:
            problems.append(
                MESSAGE_MISSING.format(
                    path=relative, label=f"optionalDependencies on {NODE_PLATFORM_SCOPE}*"
                )
            )
        for name, requirement in sorted(platform_requirements.items()):
            if requirement != expected:
                problems.append(
                    MESSAGE_MISMATCH.format(
                        path=relative,
                        label=f"optionalDependencies['{name}']",
                        actual=requirement,
                        expected=expected,
                    )
                )

    platform_root = root / NODE_PLATFORM_DIRECTORY
    platform_manifests = sorted(platform_root.glob("*/package.json"))
    if not platform_manifests:
        problems.append(
            MESSAGE_MISSING.format(path=NODE_PLATFORM_DIRECTORY, label="platform manifest")
        )
    for platform_manifest in platform_manifests:
        relative = platform_manifest.relative_to(root).as_posix()
        contents = _read_json(platform_manifest, relative, problems)
        if contents is None:
            continue
        version = contents.get("version")
        if version is None:
            problems.append(MESSAGE_MISSING.format(path=relative, label="version"))
        elif version != expected:
            problems.append(
                MESSAGE_MISMATCH.format(
                    path=relative, label="version", actual=version, expected=expected
                )
            )

    # pnpm resolves platform packages with `--frozen-lockfile`, so a stale specifier is fatal.
    relative = "bindings/node/pnpm-lock.yaml"
    if (text := _read_text(root / relative, relative, problems)) is not None:
        specifiers = re.findall(
            rf"'({re.escape(NODE_PLATFORM_SCOPE)}[a-z0-9-]+)':\s*\n\s+specifier:\s*(\S+)", text
        )
        if not specifiers:
            problems.append(
                MESSAGE_MISSING.format(
                    path=relative, label=f"specifier for {NODE_PLATFORM_SCOPE}*"
                )
            )
        for name, specifier in specifiers:
            if specifier != expected:
                problems.append(
                    MESSAGE_MISMATCH.format(
                        path=relative,
                        label=f"specifier for {name}",
                        actual=specifier,
                        expected=expected,
                    )
                )

    # Intra-workspace requirements release in lockstep and are pinned exactly. The skip list is
    # applied to the path relative to the root: `root.glob` yields absolute paths, so matching
    # against every ancestor component would disable the whole check on a runner whose workspace
    # happens to sit under a directory named `target`.
    exact_requirement = re.compile(r'(cellrune[a-z-]*)\s*=\s*\{[^}]*version\s*=\s*"=([^"]+)"')
    checked_manifests = 0
    for manifest_path in sorted(root.glob("**/Cargo.toml")):
        relative_path = manifest_path.relative_to(root)
        if SKIPPED_DIRECTORIES.intersection(relative_path.parts):
            continue
        relative = relative_path.as_posix()
        text = _read_text(manifest_path, relative, problems)
        if text is None:
            continue
        checked_manifests += 1
        for name, requirement in exact_requirement.findall(text):
            if requirement != expected:
                problems.append(
                    MESSAGE_MISMATCH.format(
                        path=relative,
                        label=f"exact requirement on {name}",
                        actual=requirement,
                        expected=expected,
                    )
                )
    if not checked_manifests:
        problems.append(MESSAGE_MISSING.format(path="Cargo.toml", label="workspace manifest"))

    # The packaged-consumer manifest resolves the crate from the directory `cargo package` writes,
    # whose name carries the version. That literal is not a requirement, so the pin check above
    # cannot see it, and a bump that misses it fails the `package` job with a missing manifest.
    relative = "release-tests/package-consumer/Cargo.toml"
    if (text := _read_text(root / relative, relative, problems)) is not None:
        _check_matches(
            text,
            relative,
            re.compile(r'path\s*=\s*"[^"]*/cellrune-([0-9][^"/]*)"'),
            "packaged crate directory",
            expected,
            problems,
        )

    relative = "release-tests/package-consumer/Cargo.lock"
    if (text := _read_text(root / relative, relative, problems)) is not None:
        _check_matches(
            text,
            relative,
            re.compile(r'^name = "cellrune"\nversion = "([^"]+)"', re.MULTILINE),
            "locked cellrune version",
            expected,
            problems,
        )

    # The generated napi shim embeds the version it enforces at load time, once in the comparison
    # and once in the message that comparison raises. Checking only the comparison would let a
    # regenerated shim tell a user to reinstall to reach the previous release.
    relative = "bindings/node/native.js"
    if (text := _read_text(root / relative, relative, problems)) is not None:
        _check_matches(
            text,
            relative,
            re.compile(r"""bindingPackageVersion !== ['"](\d+\.\d+\.\d+)['"]"""),
            "enforced binding version",
            expected,
            problems,
        )
        _check_matches(
            text,
            relative,
            re.compile(r"version mismatch, expected (\d+\.\d+\.\d+) but got"),
            "enforced binding version message",
            expected,
            problems,
        )

    # Documentation and policy prose. These ship to the registry project pages.
    for relative, pattern, label in PROSE_VERSION_PATTERNS:
        if (text := _read_text(root / relative, relative, problems)) is not None:
            _check_matches(
                text, relative, re.compile(pattern, re.MULTILINE), label, expected, problems
            )

    # The changelog drives the release notes, so the version must have its own dated section.
    relative = "CHANGELOG.md"
    if (text := _read_text(root / relative, relative, problems)) is not None:
        if not re.search(
            rf"^## \[{re.escape(expected)}\] - \d{{4}}-\d{{2}}-\d{{2}}$", text, re.MULTILINE
        ):
            problems.append(
                MESSAGE_MISSING.format(path=relative, label=f"dated [{expected}] section")
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
