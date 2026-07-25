#!/usr/bin/env python3
"""Regression tests for binding license generation helpers."""

from __future__ import annotations

import pathlib
import unittest
from unittest.mock import patch

import generate_third_party_licenses as licenses


class ResolveTargetTests(unittest.TestCase):
    def test_host_target_is_read_from_rustc_verbose_version(self) -> None:
        rustc_output = "\n".join(
            (
                "rustc 1.92.0 (example 2026-01-01)",
                "binary: rustc",
                "host: aarch64-apple-darwin",
                "release: 1.92.0",
            )
        )
        with patch.object(licenses, "command_output", return_value=rustc_output):
            target = licenses.resolve_target("host", pathlib.Path("/repository"))

        self.assertEqual(target, "aarch64-apple-darwin")

    def test_explicit_target_does_not_invoke_rustc(self) -> None:
        with patch.object(licenses, "command_output") as command:
            target = licenses.resolve_target(
                "x86_64-unknown-linux-gnu", pathlib.Path("/repository")
            )

        self.assertEqual(target, "x86_64-unknown-linux-gnu")
        command.assert_not_called()


if __name__ == "__main__":
    unittest.main()
