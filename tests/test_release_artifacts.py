"""Test release archive creation with the local native executable."""

from __future__ import annotations

import hashlib
import os
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import unittest
import zipfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
BUILDER = ROOT / "scripts" / "release-artifacts.py"
PACKAGED_README = ROOT / "packages" / "native" / "README.md"
with (ROOT / "Cargo.toml").open("rb") as source:
    PACKAGE = tomllib.load(source)["package"]
PACKAGE_NAME = PACKAGE["name"]
PACKAGE_VERSION = PACKAGE["version"]


def local_target() -> str | None:
    details = subprocess.check_output(["rustc", "-vV"], text=True)
    for line in details.splitlines():
        if line.startswith("host: "):
            target = line.removeprefix("host: ")
            if target in {
                "aarch64-apple-darwin",
                "x86_64-unknown-linux-gnu",
                "x86_64-pc-windows-msvc",
            }:
                return target
    return None


class ReleaseArtifactTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="perfect-doc-release-")
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        binary_name = PACKAGE_NAME + (".exe" if os.name == "nt" else "")
        release_binary = os.environ.get("PERFECT_DOC_RELEASE_BINARY")
        if release_binary:
            self.binary = Path(release_binary)
        else:
            cargo_target = os.environ.get("CARGO_BUILD_TARGET")
            release_dir = ROOT / "target"
            if cargo_target:
                release_dir /= cargo_target
            self.binary = release_dir / "release" / binary_name
        self.target = local_target()

    def run_builder(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(BUILDER), *args],
            cwd=ROOT,
            capture_output=True,
            text=True,
        )

    def test_rejects_an_unsupported_target(self) -> None:
        result = self.run_builder(
            "--target",
            "aarch64-unknown-linux-gnu",
            "--binary",
            str(self.binary),
            "--out-dir",
            str(self.directory),
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn("unsupported target", result.stderr)

    def test_rejects_a_tag_that_does_not_match_cargo_version(self) -> None:
        result = self.run_builder(
            "--target",
            self.target or "aarch64-apple-darwin",
            "--binary",
            str(self.binary),
            "--tag",
            "v9.9.9",
            "--out-dir",
            str(self.directory),
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn(f"tag must be v{PACKAGE_VERSION}", result.stderr)

    def test_native_archive_extracts_and_passes_runtime_checks(self) -> None:
        if self.target is None:
            self.skipTest("no supported native target is available on this host")
        if not self.binary.is_file():
            self.skipTest("build the local release binary first")

        notices = self.directory / "THIRD-PARTY-NOTICES.txt"
        notice_bytes = b"Third-party dependency notices.\nGenerated for this test.\n"
        notices.write_bytes(notice_bytes)
        result = self.run_builder(
            "--target",
            self.target,
            "--binary",
            str(self.binary),
            "--notices",
            str(notices),
            "--tag",
            f"v{PACKAGE_VERSION}",
            "--out-dir",
            str(self.directory),
        )
        self.assertEqual(result.returncode, 0, result.stderr)

        extension = ".zip" if self.target == "x86_64-pc-windows-msvc" else ".tar.gz"
        archive = self.directory / f"{PACKAGE_NAME}-{PACKAGE_VERSION}-{self.target}{extension}"
        checksum_file = archive.with_name(archive.name + ".sha256")
        checksum_line = checksum_file.read_text(encoding="ascii").strip()
        expected_line = f"{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}"
        self.assertEqual(checksum_line, expected_line)

        if extension == ".zip":
            with zipfile.ZipFile(archive) as package:
                names = set(package.namelist())
                root = f"{PACKAGE_NAME}-{PACKAGE_VERSION}-{self.target}"
                self.assertEqual(
                    names,
                    {
                        f"{root}/{PACKAGE_NAME}.exe",
                        f"{root}/LICENSE",
                        f"{root}/README.md",
                        f"{root}/THIRD-PARTY-NOTICES.txt",
                    },
                )
                mode = package.getinfo(f"{root}/{PACKAGE_NAME}.exe").external_attr >> 16
                self.assertTrue(mode & 0o111)
                self.assertEqual(
                    package.read(f"{root}/THIRD-PARTY-NOTICES.txt"), notice_bytes
                )
                self.assertEqual(
                    package.read(f"{root}/README.md"), PACKAGED_README.read_bytes()
                )
        else:
            with tarfile.open(archive, "r:gz") as package:
                members = {member.name: member for member in package.getmembers()}
                root = f"{PACKAGE_NAME}-{PACKAGE_VERSION}-{self.target}"
                self.assertEqual(
                    set(members),
                    {
                        f"{root}/{PACKAGE_NAME}",
                        f"{root}/LICENSE",
                        f"{root}/README.md",
                        f"{root}/THIRD-PARTY-NOTICES.txt",
                    },
                )
                binary_member = members[f"{root}/{PACKAGE_NAME}"]
                self.assertEqual(binary_member.mode & 0o777, 0o755)
                self.assertEqual((binary_member.uid, binary_member.gid), (0, 0))
                self.assertEqual((binary_member.uname, binary_member.gname), ("", ""))
                self.assertEqual(binary_member.mtime, 0)
                notice_member = package.extractfile(f"{root}/THIRD-PARTY-NOTICES.txt")
                self.assertIsNotNone(notice_member)
                self.assertEqual(notice_member.read(), notice_bytes)
                readme_member = package.extractfile(f"{root}/README.md")
                self.assertIsNotNone(readme_member)
                self.assertEqual(readme_member.read(), PACKAGED_README.read_bytes())


if __name__ == "__main__":
    unittest.main()
