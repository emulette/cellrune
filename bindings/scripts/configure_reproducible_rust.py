#!/usr/bin/env python3
"""Configure path-remapped Rust release builds in GitHub Actions."""

from __future__ import annotations

import os
import pathlib
import subprocess
import sys
import tempfile

ERROR_PREFIX = "reproducible Rust configuration failed"
MESSAGE_GITHUB_ENV_MISSING = "GITHUB_ENV is not set"
MESSAGE_COMMIT_TIMESTAMP_INVALID = "git commit timestamp is not numeric"


def append_environment(name: str, value: str) -> None:
    github_environment = os.environ.get("GITHUB_ENV")
    if github_environment is None:
        raise RuntimeError(MESSAGE_GITHUB_ENV_MISSING)
    with pathlib.Path(github_environment).open("a", encoding="utf-8") as output:
        output.write(f"{name}={value}\n")


def main() -> int:
    repository_root = pathlib.Path(__file__).resolve().parents[2]
    cargo_home = pathlib.Path(
        os.environ.get("CARGO_HOME", pathlib.Path.home() / ".cargo")
    ).resolve()
    temporary_root = pathlib.Path(tempfile.gettempdir()).resolve()
    source_date_epoch = subprocess.run(
        ["git", "log", "-1", "--format=%ct"],
        cwd=repository_root,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
    ).stdout.strip()
    if not source_date_epoch.isdecimal():
        raise RuntimeError(MESSAGE_COMMIT_TIMESTAMP_INVALID)

    rustflags = " ".join(
        (
            f"--remap-path-prefix={repository_root}=cellrune",
            f"--remap-path-prefix={cargo_home}=cargo-home",
            f"--remap-path-prefix={temporary_root}=temporary-build",
        )
    )
    append_environment("CARGO_INCREMENTAL", "0")
    append_environment("RUSTFLAGS", rustflags)
    append_environment("SOURCE_DATE_EPOCH", source_date_epoch)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"{ERROR_PREFIX}: {error}", file=sys.stderr)
        raise SystemExit(1) from error
