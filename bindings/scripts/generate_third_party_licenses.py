#!/usr/bin/env python3
"""Generate exact third-party license bundles for native bindings."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import subprocess
import sys
import textwrap
from dataclasses import dataclass

ERROR_PREFIX = "third-party license generation failed"
MESSAGE_TARGET_ARGUMENTS_REQUIRED = (
    "--package, --target, and --output must be supplied together"
)
MESSAGE_STALE_NOTICE = "stale {path}"
ERROR_MESSAGES = {
    "metadata.root": "cargo metadata root is not an object",
    "metadata.packages": "cargo metadata packages field is not an array",
    "metadata.package": "cargo metadata package is not an object",
    "metadata.incomplete": "registry package metadata is incomplete",
    "graph.empty": "{package} has no third-party dependencies",
    "license.vcs_metadata": "{component} VCS metadata is not an object",
    "license.revision": "{component} fallback license revision needs review",
    "license.version": "{component} supplemental license version needs review",
    "license.missing": "{component} has no distributable license file",
    "target.host": "rustc did not report a host target",
}
PACKAGE_PATTERN = re.compile(r"^(?P<name>[^ ]+) v(?P<version>[^ ]+)")
HOST_PATTERN = re.compile(r"^host: (?P<target>\S+)$", re.MULTILINE)
LICENSE_PREFIXES = ("LICENSE", "COPYING", "NOTICE", "COPYRIGHT")
# Fallback paths are repository-relative. Each entry pins the crate's VCS revision so a new
# release cannot silently reuse an unreviewed license text; r-efi declares Apache-2.0 as one of
# its alternatives but does not include a license file in the published crate.
FALLBACK_LICENSES = {
    "napi": (
        "956e4525fea6a676ea3680b711382f167b899af9",
        "bindings/licenses/napi-rs-LICENSE",
    ),
    "napi-derive": (
        "e8e3bffa2dfa77a34b8c9cbd42ea4bfef0c29729",
        "bindings/licenses/napi-rs-LICENSE",
    ),
    "napi-derive-backend": (
        "956e4525fea6a676ea3680b711382f167b899af9",
        "bindings/licenses/napi-rs-LICENSE",
    ),
    "napi-sys": (
        "679eb79f5cf3c7c6b2850f4ab46092126f23dc5c",
        "bindings/licenses/napi-rs-LICENSE",
    ),
    "r-efi": (
        "7e1b0322d31d625f81a5656096330934f9cd835d",
        "LICENSE-APACHE",
    ),
    "rmcp": (
        "1f9358eddca42d3a510c70ae6446dd6548c7c856",
        "bindings/licenses/rmcp-LICENSE",
    ),
    "rmcp-macros": (
        "1f9358eddca42d3a510c70ae6446dd6548c7c856",
        "bindings/licenses/rmcp-LICENSE",
    ),
}
SUPPLEMENTAL_LICENSES = {
    "pcre2-sys": (
        ("0.2.10", "PCRE2-10.46-LICENSE"),
        ("0.2.10", "SLJIT-LICENSE"),
    ),
}
PACKAGE_FEATURES = {
    "cellrune-python": ("extension-module",),
}


@dataclass(frozen=True, order=True)
class Component:
    name: str
    version: str
    license_expression: str
    manifest_path: pathlib.Path

    @property
    def label(self) -> str:
        return f"{self.name} {self.version}"


@dataclass
class LicenseText:
    content: str
    sources: set[str]
    components: set[str]


@dataclass(frozen=True)
class Bundle:
    cargo_package: str
    destination: pathlib.Path


def generation_error(code: str, **context: object) -> RuntimeError:
    template = ERROR_MESSAGES[code]
    return RuntimeError(f"{code}: {template.format(**context)}")


def command_output(arguments: list[str], cwd: pathlib.Path) -> str:
    completed = subprocess.run(
        arguments,
        cwd=cwd,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    return completed.stdout


def resolve_target(value: str, repository_root: pathlib.Path) -> str:
    if value != "host":
        return value
    match = HOST_PATTERN.search(command_output(["rustc", "-vV"], repository_root))
    if match is None:
        raise generation_error("target.host")
    return match.group("target")


def cargo_packages(repository_root: pathlib.Path) -> dict[tuple[str, str], Component]:
    raw_metadata: object = json.loads(
        command_output(
            ["cargo", "metadata", "--locked", "--format-version", "1"],
            repository_root,
        )
    )
    if not isinstance(raw_metadata, dict):
        raise generation_error("metadata.root")
    raw_packages = raw_metadata.get("packages")
    if not isinstance(raw_packages, list):
        raise generation_error("metadata.packages")

    packages: dict[tuple[str, str], Component] = {}
    for raw_package in raw_packages:
        if not isinstance(raw_package, dict):
            raise generation_error("metadata.package")
        name = raw_package.get("name")
        version = raw_package.get("version")
        license_expression = raw_package.get("license")
        manifest_path = raw_package.get("manifest_path")
        source = raw_package.get("source")
        if source is None:
            continue
        if not all(
            isinstance(value, str)
            for value in (name, version, license_expression, manifest_path)
        ):
            raise generation_error("metadata.incomplete")
        component = Component(
            name=name,
            version=version,
            license_expression=license_expression,
            manifest_path=pathlib.Path(manifest_path),
        )
        packages[(name, version)] = component
    return packages


def dependency_components(
    repository_root: pathlib.Path,
    cargo_package: str,
    available: dict[tuple[str, str], Component],
    target: str,
) -> list[Component]:
    feature_arguments: list[str] = []
    features = PACKAGE_FEATURES.get(cargo_package, ())
    if features:
        feature_arguments = ["--features", ",".join(features)]
    tree = command_output(
        [
            "cargo",
            "tree",
            "--locked",
            "--target",
            target,
            "--package",
            cargo_package,
            "--edges",
            "normal,no-proc-macro",
            "--prefix",
            "none",
            "--format",
            "{p}",
            *feature_arguments,
        ],
        repository_root,
    )
    selected: set[Component] = set()
    for line in tree.splitlines():
        match = PACKAGE_PATTERN.match(line.removesuffix(" (*)"))
        if match is None:
            continue
        key = (match.group("name"), match.group("version"))
        component = available.get(key)
        if component is not None:
            selected.add(component)
    if not selected:
        raise generation_error("graph.empty", package=cargo_package)
    return sorted(selected)


def component_license_files(
    component: Component,
    repository_root: pathlib.Path,
) -> list[pathlib.Path]:
    package_root = component.manifest_path.parent
    files = sorted(
        path
        for path in package_root.iterdir()
        if path.is_file() and path.name.upper().startswith(LICENSE_PREFIXES)
    )
    fallback = FALLBACK_LICENSES.get(component.name)
    if not files and fallback is not None:
        expected_revision, relative_path = fallback
        vcs_info_path = package_root / ".cargo_vcs_info.json"
        raw_vcs_info: object = json.loads(vcs_info_path.read_text(encoding="utf-8"))
        if not isinstance(raw_vcs_info, dict):
            raise generation_error(
                "license.vcs_metadata", component=component.label
            )
        git = raw_vcs_info.get("git")
        if not isinstance(git, dict) or git.get("sha1") != expected_revision:
            raise generation_error(
                "license.revision", component=component.label
            )
        files = [repository_root / relative_path]
    supplemental = SUPPLEMENTAL_LICENSES.get(component.name)
    if supplemental is not None:
        for expected_version, filename in supplemental:
            if component.version != expected_version:
                raise generation_error("license.version", component=component.label)
            files.append(repository_root / "bindings/licenses" / filename)
    if not files:
        raise generation_error("license.missing", component=component.label)
    return files


def collect_license_texts(
    components: list[Component],
    repository_root: pathlib.Path,
) -> list[LicenseText]:
    by_digest: dict[str, LicenseText] = {}
    for component in components:
        for path in component_license_files(component, repository_root):
            content = path.read_text(encoding="utf-8").strip()
            digest = hashlib.sha256(content.encode("utf-8")).hexdigest()
            record = by_digest.setdefault(
                digest,
                LicenseText(content=content, sources=set(), components=set()),
            )
            record.sources.add(path.name)
            record.components.add(component.label)
    return sorted(
        by_digest.values(),
        key=lambda record: (sorted(record.components), sorted(record.sources)),
    )


def wrapped_labels(values: set[str]) -> str:
    return "\n".join(
        textwrap.wrap(
            ", ".join(f"`{value}`" for value in sorted(values)),
            width=100,
            subsequent_indent="  ",
        )
    )


def render_bundle(
    cargo_package: str,
    components: list[Component],
    license_texts: list[LicenseText],
    target: str,
) -> str:
    graph_description = (
        "This checked-in file is a conservative all-target union. It may list"
        "\ncomponents that are not linked into a particular target artifact."
        if target == "all"
        else f"Artifact target: `{target}`"
    )
    lines = [
        "# Third-party licenses",
        "",
        "CellRune is distributed under the MIT OR Apache-2.0 licenses. Native release",
        "artifacts use the third-party components below under their own licenses.",
        "",
        graph_description,
        "",
        "This file is generated from the locked normal dependency graph for",
        f"`{cargo_package}`. Build-only and development-only dependencies",
        "are not part of this binary notice.",
        "",
        "## Components",
        "",
        "| Component | Version | SPDX expression |",
        "| --- | --- | --- |",
    ]
    lines.extend(
        f"| `{component.name}` | `{component.version}` | `{component.license_expression}` |"
        for component in components
    )
    lines.extend(["", "## License texts", ""])
    for index, record in enumerate(license_texts, start=1):
        lines.extend(
            [
                f"### License text {index}",
                "",
                f"Components: {wrapped_labels(record.components)}",
                "",
                f"Source filenames: {wrapped_labels(record.sources)}",
                "",
                "```text",
                record.content,
                "```",
                "",
            ]
        )
    return "\n".join(lines)


def configured_bundles(repository_root: pathlib.Path) -> tuple[Bundle, ...]:
    return (
        Bundle(
            "cellrune-node",
            repository_root / "bindings/node/THIRD_PARTY_LICENSES.md",
        ),
        Bundle(
            "cellrune-python",
            repository_root / "bindings/python/THIRD_PARTY_LICENSES.md",
        ),
        Bundle(
            "cellrune-mcp",
            repository_root / "crates/cellrune-mcp/THIRD_PARTY_LICENSES.md",
        ),
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--check",
        action="store_true",
        help="fail when a checked-in notice differs from the locked graph",
    )
    parser.add_argument("--package", dest="cargo_package")
    parser.add_argument("--target")
    parser.add_argument("--output", type=pathlib.Path)
    arguments = parser.parse_args()
    repository_root = pathlib.Path(__file__).resolve().parents[2]
    available = cargo_packages(repository_root)
    stale: list[pathlib.Path] = []

    targeted = (arguments.cargo_package, arguments.target, arguments.output)
    if any(value is not None for value in targeted) and not all(
        value is not None for value in targeted
    ):
        parser.error(MESSAGE_TARGET_ARGUMENTS_REQUIRED)
    bundles = (
        (
            Bundle(arguments.cargo_package, arguments.output),
            resolve_target(arguments.target, repository_root),
        ),
    ) if all(value is not None for value in targeted) else tuple(
        (bundle, "all") for bundle in configured_bundles(repository_root)
    )

    for bundle, target in bundles:
        assert target is not None
        components = dependency_components(
            repository_root, bundle.cargo_package, available, target
        )
        license_texts = collect_license_texts(components, repository_root)
        expected = render_bundle(
            bundle.cargo_package, components, license_texts, target
        ).encode("utf-8")
        if arguments.check:
            actual = (
                bundle.destination.read_bytes()
                if bundle.destination.exists()
                else b""
            )
            if actual != expected:
                stale.append(bundle.destination)
        else:
            bundle.destination.write_bytes(expected)

    if stale:
        for path in stale:
            message = MESSAGE_STALE_NOTICE.format(
                path=path.relative_to(repository_root)
            )
            print(f"{ERROR_PREFIX}: {message}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"{ERROR_PREFIX}: {error}", file=sys.stderr)
        raise SystemExit(1) from error
