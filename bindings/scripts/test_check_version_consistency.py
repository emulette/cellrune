#!/usr/bin/env python3
"""Regression tests for the version consistency gate.

The gate is the only thing standing between a partial version bump and a release run that fails
after its irreversible approvals, so its own failure mode matters as much as its checks: a pattern
that stops matching, or a file that moves, must fail rather than report the same green result as a
correct bump. Each test below drifts exactly one declaration in a copy of the real repository and
asserts the gate names it.
"""

from __future__ import annotations

import json
import shutil
import tempfile
import unittest
from pathlib import Path

from check_version_consistency import check
from workspace_version import repository_root, workspace_version

SKIPPED_TREES = shutil.ignore_patterns("target", "node_modules", ".git")


class VersionConsistencyTests(unittest.TestCase):
    """Each test copies the repository, drifts one declaration, and checks the gate reports it."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.source = repository_root()
        cls.expected = workspace_version(cls.source)
        cls.stale = "0.0.1"

    def tree(self) -> Path:
        """Return a writable copy of the repository that is removed when the test ends."""
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name) / "cellrune"
        shutil.copytree(self.source, root, ignore=SKIPPED_TREES, symlinks=True)
        return root

    def assert_reports(self, root: Path, fragment: str) -> None:
        problems = check(root, self.expected)
        self.assertTrue(
            any(fragment in problem for problem in problems),
            f"expected a problem mentioning {fragment!r}, got {problems}",
        )

    def test_unmodified_repository_passes(self) -> None:
        self.assertEqual(check(self.tree(), self.expected), [])

    def test_checkout_under_a_skipped_directory_still_checks_cargo_pins(self) -> None:
        # The skip list applies to the repository-relative path. Matching against every ancestor
        # component instead would disable the whole check on a runner whose workspace happens to
        # live under a directory named `target`, which is where a self-hosted runner often puts it.
        root = self.tree()
        nested = root.parent / "target" / "cellrune"
        nested.parent.mkdir(parents=True)
        shutil.move(str(root), str(nested))
        manifest = nested / "bindings/common/Cargo.toml"
        manifest.write_text(
            manifest.read_text(encoding="utf-8").replace(
                f'version = "={self.expected}"', f'version = "={self.stale}"', 1
            ),
            encoding="utf-8",
        )
        self.assert_reports(nested, "exact requirement on cellrune")

    def test_third_party_optional_dependency_is_allowed(self) -> None:
        # Only the workspace's own platform packages track the release version. Requiring every
        # optional dependency to equal it would block any pull request that added an ordinary one.
        root = self.tree()
        path = root / "bindings/node/package.json"
        manifest = json.loads(path.read_text(encoding="utf-8"))
        manifest["optionalDependencies"]["fsevents"] = "^2.3.3"
        path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
        self.assertEqual(check(root, self.expected), [])

    def test_regenerated_native_shim_quoting_is_not_a_silent_pass(self) -> None:
        # A napi-rs upgrade that changes the generated shim's quoting would leave the old pattern
        # matching nothing. Reporting success then would ship a loader that rejects its own binary.
        root = self.tree()
        path = root / "bindings/node/native.js"
        path.write_text(
            path.read_text(encoding="utf-8").replace(
                f"bindingPackageVersion !== '{self.expected}'",
                f'bindingPackageVersion !== "{self.stale}"',
            ),
            encoding="utf-8",
        )
        self.assert_reports(root, "enforced binding version")

    def test_missing_native_shim_is_reported(self) -> None:
        root = self.tree()
        (root / "bindings/node/native.js").unlink()
        self.assert_reports(root, "bindings/node/native.js")

    def test_stale_native_shim_message_is_reported(self) -> None:
        # The shim carries the version twice per platform branch: in the comparison and in the
        # message it raises. A bump that updates only the comparison tells a user who hits a real
        # mismatch to reinstall to reach the previous release.
        root = self.tree()
        path = root / "bindings/node/native.js"
        path.write_text(
            path.read_text(encoding="utf-8").replace(
                f"version mismatch, expected {self.expected} but got",
                f"version mismatch, expected {self.stale} but got",
            ),
            encoding="utf-8",
        )
        self.assert_reports(root, "enforced binding version message")

    def test_stale_packaged_consumer_path_is_reported(self) -> None:
        # The version inside the `path` value is not a requirement, so the exact-pin check cannot
        # see it. A bump that misses it fails the `package` job with a missing manifest.
        root = self.tree()
        path = root / "release-tests/package-consumer/Cargo.toml"
        path.write_text(
            path.read_text(encoding="utf-8").replace(
                f"cellrune-{self.expected}", f"cellrune-{self.stale}"
            ),
            encoding="utf-8",
        )
        self.assert_reports(root, "packaged crate directory")

    def test_stale_packaged_consumer_lockfile_is_reported(self) -> None:
        root = self.tree()
        path = root / "release-tests/package-consumer/Cargo.lock"
        path.write_text(
            path.read_text(encoding="utf-8").replace(
                f'name = "cellrune"\nversion = "{self.expected}"',
                f'name = "cellrune"\nversion = "{self.stale}"',
            ),
            encoding="utf-8",
        )
        self.assert_reports(root, "locked cellrune version")

    def test_stale_install_instructions_are_reported(self) -> None:
        # These literals ship on the crates.io and PyPI project pages and the repository front
        # page, where a stale one tells every reader to install a version that is not current.
        root = self.tree()
        path = root / "README.md"
        path.write_text(
            path.read_text(encoding="utf-8").replace(
                f"cargo add cellrune@{self.expected}", f"cargo add cellrune@{self.stale}"
            ),
            encoding="utf-8",
        )
        self.assert_reports(root, "cargo add version")

    def test_stale_security_policy_version_is_reported(self) -> None:
        root = self.tree()
        path = root / "SECURITY.md"
        path.write_text(
            path.read_text(encoding="utf-8").replace(
                f"CellRune is at `{self.expected}`", f"CellRune is at `{self.stale}`"
            ),
            encoding="utf-8",
        )
        self.assert_reports(root, "supported version")

    def test_stale_roadmap_current_release_is_reported(self) -> None:
        # The roadmap separates planned work from what already shipped, and it anchors that
        # distinction on one literal. A stale one moves the boundary: work released in the
        # current version keeps reading as planned.
        root = self.tree()
        path = root / "docs/ROADMAP.md"
        path.write_text(
            path.read_text(encoding="utf-8").replace(
                f"Current release: **{self.expected}**",
                f"Current release: **{self.stale}**",
            ),
            encoding="utf-8",
        )
        self.assert_reports(root, "roadmap current release")

    def test_missing_platform_manifests_are_reported(self) -> None:
        root = self.tree()
        shutil.rmtree(root / "bindings/node/npm")
        self.assert_reports(root, "platform manifest")

    def test_malformed_manifest_is_reported_rather_than_raised(self) -> None:
        root = self.tree()
        path = root / "bindings/node/package.json"
        path.write_text('{"version": ', encoding="utf-8")
        self.assert_reports(root, "cannot be read")

    def test_missing_changelog_section_is_reported(self) -> None:
        root = self.tree()
        path = root / "CHANGELOG.md"
        path.write_text(
            path.read_text(encoding="utf-8").replace(f"## [{self.expected}] - ", "## [x] - "),
            encoding="utf-8",
        )
        self.assert_reports(root, "dated")


if __name__ == "__main__":
    unittest.main()
