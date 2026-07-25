#!/usr/bin/env python3
"""Strip non-runtime symbols from native binding artifacts."""

from __future__ import annotations

import argparse
import os
import pathlib
import re
import subprocess
import sys

ERROR_PREFIX = "native artifact stripping failed"
MESSAGE_ARTIFACT_MISSING = "artifact does not exist: {path}"
MESSAGE_ARTIFACT_TYPE = "artifact is not a supported native library: {path}"
MESSAGE_ARTIFACT_EMPTY = "stripped artifact is empty: {path}"
MESSAGE_RUSTC_HOST_MISSING = "rustc did not report a host target"
MESSAGE_RUSTC_OUTPUT_EMPTY = "rustc returned an empty value for {name}"
MESSAGE_RUST_LIBRARY_DIR_MISSING = "Rust toolchain library directory is unavailable: {path}"
MESSAGE_RUST_OBJCOPY_MISSING = "Rust toolchain objcopy is unavailable: {path}"
SUPPORTED_SUFFIXES = frozenset({".dylib", ".exe", ".node", ".pyd", ".so"})


def rustc_output(arguments: list[str], name: str) -> str:
    completed = subprocess.run(
        ["rustc", *arguments],
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    value = completed.stdout.strip()
    if not value:
        raise RuntimeError(MESSAGE_RUSTC_OUTPUT_EMPTY.format(name=name))
    return value


def rustc_host() -> str:
    verbose_version = rustc_output(["-vV"], "verbose version")
    match = re.search(r"^host: (?P<host>\S+)$", verbose_version, re.MULTILINE)
    if match is None:
        raise RuntimeError(MESSAGE_RUSTC_HOST_MISSING)
    return match.group("host")


def rust_objcopy_path(sysroot: pathlib.Path, host: str) -> pathlib.Path:
    executable = "rust-objcopy.exe" if os.name == "nt" else "rust-objcopy"
    path = sysroot / "lib" / "rustlib" / host / "bin" / executable
    if not path.is_file():
        raise RuntimeError(MESSAGE_RUST_OBJCOPY_MISSING.format(path=path))
    return path


def tool_environment(sysroot: pathlib.Path) -> dict[str, str]:
    environment = os.environ.copy()
    if sys.platform.startswith("linux"):
        library_directory = sysroot / "lib"
        if not library_directory.is_dir():
            raise RuntimeError(
                MESSAGE_RUST_LIBRARY_DIR_MISSING.format(path=library_directory)
            )
        existing = environment.get("LD_LIBRARY_PATH")
        environment["LD_LIBRARY_PATH"] = (
            f"{library_directory}{os.pathsep}{existing}"
            if existing
            else str(library_directory)
        )
    return environment


def strip_artifact(
    tool: pathlib.Path,
    environment: dict[str, str],
    artifact: pathlib.Path,
) -> None:
    if not artifact.is_file():
        raise RuntimeError(MESSAGE_ARTIFACT_MISSING.format(path=artifact))
    if artifact.suffix.lower() not in SUPPORTED_SUFFIXES:
        raise RuntimeError(MESSAGE_ARTIFACT_TYPE.format(path=artifact))

    original_size = artifact.stat().st_size
    subprocess.run(
        [tool, "--strip-all", artifact],
        check=True,
        env=environment,
    )
    stripped_size = artifact.stat().st_size
    if stripped_size == 0:
        raise RuntimeError(MESSAGE_ARTIFACT_EMPTY.format(path=artifact))
    print(f"stripped {artifact}: {original_size} -> {stripped_size} bytes")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("artifacts", nargs="+", type=pathlib.Path)
    arguments = parser.parse_args()
    sysroot = pathlib.Path(rustc_output(["--print", "sysroot"], "sysroot"))
    tool = rust_objcopy_path(sysroot, rustc_host())
    environment = tool_environment(sysroot)
    for artifact in arguments.artifacts:
        strip_artifact(tool, environment, artifact)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"{ERROR_PREFIX}: {error}", file=sys.stderr)
        raise SystemExit(1) from error
