"""Test npm staging and its process launcher with local fixtures."""

import json
import os
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import tarfile
import tomllib
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
BUILDER = ROOT / "scripts" / "package-npm.py"
LAUNCHER = ROOT / "packages" / "npm" / "launcher.js"
with (ROOT / "Cargo.toml").open("rb") as source:
    PACKAGE_VERSION = tomllib.load(source)["package"]["version"]


def elf_fixture(
    interpreter: str | None = "/lib64/ld-linux-x86-64.so.2", machine: int = 62
) -> bytes:
    """Return a small ELF64 fixture with one interpreter entry."""
    path = interpreter.encode("ascii") + b"\0" if interpreter else b""
    header = bytearray(64)
    header[:7] = b"\x7fELF\x02\x01\x01"
    struct.pack_into("<H", header, 18, machine)
    struct.pack_into("<Q", header, 32, 64)
    struct.pack_into("<H", header, 52, 64)
    struct.pack_into("<HH", header, 54, 56, 1 if interpreter else 0)
    program_header = bytearray(56)
    if interpreter:
        struct.pack_into("<I", program_header, 0, 3)
        struct.pack_into("<Q", program_header, 8, 120)
        struct.pack_into("<Q", program_header, 32, len(path))
        struct.pack_into("<Q", program_header, 40, len(path))
    return bytes(header + program_header) + b"\0" * (120 - 120) + path


def macho_fixture(cpu: int) -> bytes:
    return b"\xcf\xfa\xed\xfe" + struct.pack("<I", cpu) + b"\0" * 64


def pe_fixture(machine: int = 0x8664) -> bytes:
    data = bytearray(70)
    data[:2] = b"MZ"
    struct.pack_into("<I", data, 0x3C, 64)
    data[64:68] = b"PE\0\0"
    struct.pack_into("<H", data, 68, machine)
    return bytes(data)


def node_platform(node: str) -> tuple[str, str, str | None]:
    source = (
        "const r=process.report&&process.report.getReport().header;"
        "console.log(JSON.stringify({os:process.platform,cpu:process.arch,"
        "libc:process.platform==='linux'?(r.glibcVersionRuntime?'glibc':'musl'):null}))"
    )
    result = subprocess.run([node, "-e", source], check=True, capture_output=True, text=True)
    value = json.loads(result.stdout)
    return value["os"], value["cpu"], value["libc"]


class PackageBuilderTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="perfect doc npm ")
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)

    def run_builder(self, data: bytes, target: str = "x86_64-unknown-linux-gnu") -> subprocess.CompletedProcess[str]:
        binary = self.directory / "release binary"
        binary.write_bytes(data)
        stage = self.directory / "stage with spaces" / target
        return subprocess.run(
            [
                sys.executable,
                str(BUILDER),
                "--target",
                target,
                "--binary",
                str(binary),
                "--stage-dir",
                str(stage),
            ],
            cwd=ROOT,
            capture_output=True,
            text=True,
        )

    @unittest.skipUnless(shutil.which("npm"), "npm is required for pack checks")
    def test_builder_packs_private_base_and_platform_manifests(self) -> None:
        result = self.run_builder(elf_fixture())
        self.assertEqual(result.returncode, 0, result.stderr)

        stage = self.directory / "stage with spaces" / "x86_64-unknown-linux-gnu"
        base = json.loads((stage / "base" / "package.json").read_text())
        platform_manifest = json.loads(
            (stage / "platform" / "perfect-doc-linux-x64-gnu" / "package.json").read_text()
        )
        self.assertTrue(base["private"])
        self.assertTrue(platform_manifest["private"])
        self.assertEqual(base["version"], platform_manifest["version"])
        self.assertEqual(base["license"], "MIT")
        self.assertEqual(platform_manifest["license"], "MIT")
        self.assertEqual(platform_manifest["os"], ["linux"])
        self.assertEqual(platform_manifest["cpu"], ["x64"])
        self.assertEqual(platform_manifest["libc"], "glibc")
        archives = list((stage / "tarballs").glob("*.tgz"))
        self.assertEqual(len(archives), 2)
        contents = {}
        for archive in archives:
            with tarfile.open(archive, "r:gz") as package:
                contents[archive.name] = set(package.getnames())
                license_file = package.extractfile("package/LICENSE")
                self.assertIsNotNone(license_file)
                self.assertEqual(license_file.read(), (ROOT / "LICENSE").read_bytes())
        self.assertEqual(
            contents[f"perfect-doc-{PACKAGE_VERSION}.tgz"],
            {"package/package.json", "package/bin/perfect-doc.js", "package/LICENSE"},
        )
        self.assertEqual(
            contents[f"perfect-doc-linux-x64-gnu-{PACKAGE_VERSION}.tgz"],
            {"package/package.json", "package/bin/perfect-doc", "package/LICENSE"},
        )

    @unittest.skipUnless(shutil.which("npm"), "npm is required for pack checks")
    def test_builder_packs_each_supported_target_shape(self) -> None:
        cases = [
            ("aarch64-apple-darwin", macho_fixture(0x0100000C), "darwin-arm64", "arm64", None),
            ("x86_64-apple-darwin", macho_fixture(0x01000007), "darwin-x64", "x64", None),
            ("x86_64-unknown-linux-musl", elf_fixture("/lib/ld-musl-x86_64.so.1"), "linux-x64-musl", "x64", "musl"),
            ("x86_64-pc-windows-msvc", pe_fixture(), "win32-x64", "x64", None),
        ]
        for target, data, suffix, cpu, libc in cases:
            with self.subTest(target=target):
                result = self.run_builder(data, target)
                self.assertEqual(result.returncode, 0, result.stderr)
                stage = self.directory / "stage with spaces" / target
                manifest = json.loads(
                    (stage / "platform" / f"perfect-doc-{suffix}" / "package.json").read_text()
                )
                self.assertEqual(manifest["cpu"], [cpu])
                self.assertEqual(manifest.get("libc"), libc)
                self.assertEqual(len(list((stage / "tarballs").glob("*.tgz"))), 2)

    def test_builder_rejects_a_binary_with_the_wrong_architecture(self) -> None:
        result = self.run_builder(elf_fixture(machine=183))
        self.assertEqual(result.returncode, 2)
        self.assertIn("CPU signature", result.stderr)

    def test_builder_rejects_an_unverified_static_libc(self) -> None:
        result = self.run_builder(elf_fixture(interpreter=None))
        self.assertEqual(result.returncode, 2)
        self.assertIn("cannot confirm the libc", result.stderr)

    def test_builder_rejects_a_dynamic_binary_for_the_wrong_libc(self) -> None:
        result = self.run_builder(elf_fixture("/lib/ld-musl-x86_64.so.1"))
        self.assertEqual(result.returncode, 2)
        self.assertIn("interpreter does not match", result.stderr)

    def test_builder_rejects_an_unsupported_target(self) -> None:
        result = self.run_builder(elf_fixture(), "aarch64-unknown-linux-gnu")
        self.assertEqual(result.returncode, 2)
        self.assertIn("unsupported target", result.stderr)


@unittest.skipUnless(shutil.which("node"), "Node.js is required for launcher checks")
class LauncherTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.node = shutil.which("node")
        cls.os_name, cls.cpu, cls.libc = node_platform(cls.node)
        suffixes = {
            ("darwin", "arm64"): "darwin-arm64",
            ("darwin", "x64"): "darwin-x64",
            ("win32", "x64"): "win32-x64",
        }
        if cls.os_name == "linux" and cls.cpu == "x64":
            suffixes[(cls.os_name, cls.cpu)] = "linux-x64-gnu" if cls.libc == "glibc" else "linux-x64-musl"
        cls.package_name = "perfect-doc-" + suffixes.get((cls.os_name, cls.cpu), "unsupported")

    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="perfect doc launcher ")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.launcher = self.root / "bin" / "perfect-doc.js"
        self.launcher.parent.mkdir(parents=True)
        shutil.copyfile(LAUNCHER, self.launcher)
        self.package = self.root / "node_modules" / self.package_name
        self.binary = self.package / "bin" / ("perfect-doc.exe" if self.os_name == "win32" else "perfect-doc")
        self.binary.parent.mkdir(parents=True)
        self.record = self.root / "captured args.json"
        self.manifest = {
            "name": self.package_name,
            "version": PACKAGE_VERSION,
            "os": [self.os_name],
            "cpu": [self.cpu],
            "bin": {"perfect-doc": "bin/" + self.binary.name},
        }
        if self.libc:
            self.manifest["libc"] = self.libc
        self.write_manifest()
        if os.name == "nt":
            self.skipTest("process fixture uses a Unix executable script")
        script = (
            "#!/usr/bin/env python3\n"
            "import json, os, signal, sys\n"
            "open(os.environ['PERFECT_DOC_TEST_RECORD'], 'w').write(json.dumps({'args': sys.argv[1:], 'cwd': os.getcwd()}))\n"
            "if os.environ.get('PERFECT_DOC_TEST_SIGNAL'): os.kill(os.getpid(), int(os.environ['PERFECT_DOC_TEST_SIGNAL']))\n"
            "raise SystemExit(int(os.environ.get('PERFECT_DOC_TEST_EXIT', '0')))\n"
        )
        self.binary.write_text(script)
        self.binary.chmod(0o755)

    def write_manifest(self) -> None:
        self.package.mkdir(parents=True, exist_ok=True)
        (self.package / "package.json").write_text(json.dumps(self.manifest))

    def run_launcher(self, *args: str, **extra_env: str) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        env["PERFECT_DOC_TEST_RECORD"] = str(self.record)
        env.update(extra_env)
        return subprocess.run(
            [self.node, str(self.launcher), *args],
            cwd=self.root,
            env=env,
            capture_output=True,
            text=True,
        )

    def test_success_forwards_arguments_and_working_directory_with_spaces(self) -> None:
        output = self.run_launcher("argument with spaces", "another value")
        self.assertEqual(output.returncode, 0, output.stderr)
        captured = json.loads(self.record.read_text())
        self.assertEqual(captured["args"], ["argument with spaces", "another value"])
        self.assertEqual(captured["cwd"], str(self.root.resolve()))

    def test_failure_exit_status_is_preserved(self) -> None:
        output = self.run_launcher("bad input", PERFECT_DOC_TEST_EXIT="17")
        self.assertEqual(output.returncode, 17)

    @unittest.skipIf(os.name == "nt", "signal exit codes are POSIX behavior")
    def test_child_signal_is_forwarded(self) -> None:
        output = self.run_launcher(PERFECT_DOC_TEST_SIGNAL=str(signal.SIGTERM))
        self.assertEqual(output.returncode, -signal.SIGTERM)

    def test_incomplete_platform_manifest_fails(self) -> None:
        self.manifest = {"name": self.package_name, "bin": {"perfect-doc": "bin/perfect-doc"}}
        self.write_manifest()
        output = self.run_launcher()
        self.assertEqual(output.returncode, 2)
        self.assertIn("incomplete", output.stderr)

    def test_wrong_platform_manifest_fails(self) -> None:
        self.manifest["os"] = ["other-os"]
        self.write_manifest()
        output = self.run_launcher()
        self.assertEqual(output.returncode, 2)
        self.assertIn("does not match", output.stderr)

    def test_missing_binary_fails(self) -> None:
        self.binary.unlink()
        output = self.run_launcher()
        self.assertEqual(output.returncode, 2)
        self.assertIn("binary is missing", output.stderr)


if __name__ == "__main__":
    unittest.main()
