#!/usr/bin/env python3
"""Validate the stable large-workbook benchmark contract without wall-clock ceilings."""

from __future__ import annotations

import argparse
import math
from pathlib import Path

SCHEMA = "cellrune_hardening_benchmark_v3"
MINIMUM_RELEASE_ROWS = 50_000
INTEGER_METRICS = {
    "rows",
    "iterations",
    "archive_bytes",
    "output_bytes",
    "read_mean_ns",
    "scan_mean_ns",
    "calculate_mean_ns",
    "write_mean_ns",
    "reopen_and_recalculate_mean_ns",
}
FLOAT_METRICS = {
    "read_mean_ms",
    "scan_mean_ms",
    "calculate_mean_ms",
    "write_mean_ms",
    "reopen_and_recalculate_mean_ms",
}
ERROR_PREFIX = "benchmark verification failed"
ERROR_MESSAGES = {
    "missing_schema": "missing schema {schema}",
    "duplicate_metric": "duplicate metric {name}",
    "metric_schema": "metric schema mismatch; missing={missing}, unexpected={unexpected}",
    "integer": "{name} is not an integer",
    "positive": "{name} must be greater than zero",
    "rows": "rows={rows} is below {minimum}",
    "numeric": "{name} is not numeric",
    "finite": "{name} must be finite and non-negative",
}


def failure(code: str, **values: object) -> ValueError:
    return ValueError(f"{ERROR_PREFIX}: {ERROR_MESSAGES[code].format(**values)}")


def parse_output(path: Path) -> dict[str, str]:
    lines = path.read_text(encoding="utf-8").splitlines()
    try:
        schema_index = lines.index(SCHEMA)
    except ValueError as error:
        raise failure("missing_schema", schema=SCHEMA) from error

    metrics: dict[str, str] = {}
    for line in lines[schema_index + 1 :]:
        if "\t" not in line:
            continue
        name, value = line.split("\t", 1)
        if name in metrics:
            raise failure("duplicate_metric", name=name)
        metrics[name] = value
    expected = INTEGER_METRICS | FLOAT_METRICS
    if set(metrics) != expected:
        missing = sorted(expected - set(metrics))
        unexpected = sorted(set(metrics) - expected)
        raise failure("metric_schema", missing=missing, unexpected=unexpected)
    return metrics


def verify(metrics: dict[str, str], minimum_rows: int) -> None:
    integers: dict[str, int] = {}
    for name in sorted(INTEGER_METRICS):
        try:
            integers[name] = int(metrics[name])
        except ValueError as error:
            raise failure("integer", name=name) from error
        if integers[name] <= 0:
            raise failure("positive", name=name)

    if integers["rows"] < minimum_rows:
        raise failure(
            "rows",
            rows=integers["rows"],
            minimum=minimum_rows,
        )

    for name in sorted(FLOAT_METRICS):
        try:
            value = float(metrics[name])
        except ValueError as error:
            raise failure("numeric", name=name) from error
        if not math.isfinite(value) or value < 0:
            raise failure("finite", name=name)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("output", type=Path)
    parser.add_argument("--minimum-rows", type=int, default=MINIMUM_RELEASE_ROWS)
    arguments = parser.parse_args()
    if arguments.minimum_rows <= 0:
        parser.error("--minimum-rows must be greater than zero")
    verify(parse_output(arguments.output), arguments.minimum_rows)


if __name__ == "__main__":
    main()
