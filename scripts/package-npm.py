#!/usr/bin/env python3
"""Build private npm tarballs for Perfect Doc and one native target."""

from __future__ import annotations

import argparse
import json
import shutil
import struct
import subprocess
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
LAUNCHER = ROOT / "packages" / "npm" / "launcher.js"
LICENSE = ROOT / "LICENSE"


@dataclass(frozen=True)
class Target:
    triple: str
    package_suffix: str
    os_name: str
    cpu: str
    libc: str | None
    binary_name: str


TARGETS = {
    "aarch64-apple-darwin": Target(
        "aarch64-apple-darwin", "darwin-arm64", "darwin", "arm64", None, "perfect-doc"
    ),
    "x86_64-apple-darwin": Target(
        "x86_64-apple-darwin", "darwin-x64", "darwin", "x64", None, "perfect-doc"
    ),
    "x86_64-unknown-linux-gnu": Target(
        "x86_64-unknown-linux-gnu", "linux-x64-gnu", "linux", "x64", "glibc", "perfect-doc"
    ),
    "x86_64-unknown-linux-musl": Target(
        "x86_64-unknown-linux-musl", "linux-x64-musl", "linux", "x64", "musl", "perfect-doc"
    ),
    "x86_64-pc-windows-msvc": Target(
        "x86_64-pc-windows-msvc", "win32-x64", "win32", "x64", None, "perfect-doc.exe"
    ),
}


class PackageError(Exception):
    """A package build failed with a user-facing message."""


def cargo_version() -> str:
    with (ROOT / "Cargo.toml").open("rb") as source:
        return tomllib.load(source)["package"]["version"]


def cargo_license() -> str:
    with (ROOT / "Cargo.toml").open("rb") as source:
        return tomllib.load(source)["package"]["license"]


def cargo_host() -> str:
    try:
        result = subprocess.run(
            ["rustc", "-vV"], check=True, capture_output=True, text=True
        )
    except (OSError, subprocess.CalledProcessError) as error:
        raise PackageError(f"cannot read the Rust host target: {error}") from error
    for line in result.stdout.splitlines():
        if line.startswith("host: "):
            return line.removeprefix("host: ")
    raise PackageError("rustc did not report its host target")


def elf_signature(data: bytes) -> tuple[int, int, str | None]:
    if len(data) < 64 or data[:4] != b"\x7fELF":
        raise PackageError("the binary is not a supported ELF executable")
    elf_class, elf_data = data[4], data[5]
    endian = {1: "<", 2: ">"}.get(elf_data)
    if elf_class not in (1, 2) or endian is None:
        raise PackageError("the ELF binary has an unsupported class or byte order")
    if elf_class != 2:
        raise PackageError("the Linux x86_64 target needs an ELF64 binary")
    machine = struct.unpack_from(endian + "H", data, 18)[0]
    if elf_class == 2:
        phoff = struct.unpack_from(endian + "Q", data, 32)[0]
        phentsize, phnum = struct.unpack_from(endian + "HH", data, 54)
        type_offset, file_offset_offset, file_size_offset = 0, 8, 32
        word = "Q"
    else:
        phoff = struct.unpack_from(endian + "I", data, 28)[0]
        phentsize, phnum = struct.unpack_from(endian + "HH", data, 42)
        type_offset, file_offset_offset, file_size_offset = 0, 4, 16
        word = "I"

    interpreter = None
    for index in range(phnum):
        header = phoff + index * phentsize
        if header + phentsize > len(data):
            raise PackageError("the ELF binary has an incomplete program header")
        program_type = struct.unpack_from(endian + "I", data, header + type_offset)[0]
        if program_type == 3:
            file_offset = struct.unpack_from(endian + word, data, header + file_offset_offset)[0]
            file_size = struct.unpack_from(endian + word, data, header + file_size_offset)[0]
            if file_offset + file_size > len(data):
                raise PackageError("the ELF binary has an incomplete interpreter path")
            interpreter = (
                data[file_offset : file_offset + file_size]
                .split(b"\0", 1)[0]
                .decode("ascii", errors="replace")
                or None
            )
            break
    return elf_class, machine, interpreter


def binary_matches(binary: Path, target: Target, cargo_selected_target: bool) -> None:
    try:
        data = binary.read_bytes()
    except OSError as error:
        raise PackageError(f"cannot read binary {binary}: {error}") from error

    triple = target.triple
    if "apple-darwin" in triple:
        byte_orders = {b"\xcf\xfa\xed\xfe": "<", b"\xfe\xed\xfa\xcf": ">"}
        endian = byte_orders.get(data[:4])
        if endian is None or len(data) < 8:
            raise PackageError(f"binary signature does not match {triple}")
        cpu_type = struct.unpack_from(endian + "I", data, 4)[0]
        expected = 0x0100000C if target.cpu == "arm64" else 0x01000007
        if cpu_type != expected:
            raise PackageError(f"binary CPU signature does not match {triple}")
        return

    if "windows-msvc" in triple:
        if len(data) < 64 or data[:2] != b"MZ":
            raise PackageError(f"binary signature does not match {triple}")
        pe_offset = struct.unpack_from("<I", data, 0x3C)[0]
        if pe_offset + 6 > len(data) or data[pe_offset : pe_offset + 4] != b"PE\0\0":
            raise PackageError(f"binary signature does not match {triple}")
        machine = struct.unpack_from("<H", data, pe_offset + 4)[0]
        if machine != 0x8664:
            raise PackageError(f"binary CPU signature does not match {triple}")
        return

    elf_class, machine, interpreter = elf_signature(data)
    if elf_class != 2:
        raise PackageError(f"binary signature does not match {triple}")
    if machine != 62:
        raise PackageError(f"binary CPU signature does not match {triple}")
    if target.libc == "glibc":
        if interpreter and not ("ld-linux" in interpreter or "ld64.so" in interpreter):
            raise PackageError("the ELF interpreter does not match the GNU Linux target")
        if interpreter is None and not cargo_selected_target:
            raise PackageError("cannot confirm the libc for this static ELF binary")
    if target.libc == "musl":
        if interpreter and "ld-musl" not in interpreter:
            raise PackageError("the ELF interpreter does not match the musl Linux target")
        if interpreter is None and not cargo_selected_target and not (
            b"musl" in data and b"GLIBC_" not in data
        ):
            raise PackageError("cannot confirm the libc for this static ELF binary")


def package_json(target: Target, version: str, base_name: str, license_name: str) -> dict[str, object]:
    return {
        "name": f"{base_name}-{target.package_suffix}",
        "version": version,
        "license": license_name,
        "private": True,
        "os": [target.os_name],
        "cpu": [target.cpu],
        **({"libc": target.libc} if target.libc else {}),
        "files": [f"bin/{target.binary_name}", "LICENSE"],
        "bin": {"perfect-doc": f"bin/{target.binary_name}"},
    }


def write_json(path: Path, content: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(content, indent=2) + "\n", encoding="utf-8")


def build_target(target: Target, binary: Path | None) -> tuple[Path, bool]:
    if binary is not None:
        return binary.resolve(), False
    cargo = shutil.which("cargo")
    if cargo is None:
        raise PackageError("Cargo is required to build a native package binary")
    try:
        subprocess.run(
            [cargo, "build", "--locked", "--release", "--bin", "perfect-doc", "--target", target.triple],
            cwd=ROOT,
            check=True,
        )
    except subprocess.CalledProcessError as error:
        raise PackageError(f"Cargo failed to build target {target.triple}") from error
    return ROOT / "target" / target.triple / "release" / target.binary_name, True


def main() -> int:
    parser = argparse.ArgumentParser(description="Build private npm tarballs for one Rust target.")
    parser.add_argument("--target", help="Rust target triple; defaults to the rustc host")
    parser.add_argument("--binary", type=Path, help="use an existing release binary")
    parser.add_argument(
        "--stage-dir",
        type=Path,
        default=ROOT / "dist" / "npm",
        help="staging directory; defaults to dist/npm",
    )
    parser.add_argument(
        "--base-name", default="perfect-doc", help="npm package name for the launcher"
    )
    args = parser.parse_args()

    try:
        target_name = args.target or cargo_host()
        target = TARGETS.get(target_name)
        if target is None:
            raise PackageError(f"unsupported target: {target_name}")
        if args.binary is not None and args.target is None:
            raise PackageError("--target is required when --binary is used")
        binary, cargo_selected_target = build_target(target, args.binary)
        if not binary.is_file():
            raise PackageError(f"release binary does not exist: {binary}")
        binary_matches(binary, target, cargo_selected_target)

        stage = args.stage_dir.resolve()
        version = cargo_version()
        license_name = cargo_license()
        base_dir = stage / "base"
        platform_name = f"{args.base_name}-{target.package_suffix}"
        platform_dir = stage / "platform" / platform_name
        base_manifest = {
            "name": args.base_name,
            "version": version,
            "license": license_name,
            "private": True,
            "description": "Native structural documentation validator",
            "files": ["bin/perfect-doc.js", "LICENSE"],
            "bin": {"perfect-doc": "bin/perfect-doc.js"},
            "optionalDependencies": {
                f"{args.base_name}-{item.package_suffix}": version for item in TARGETS.values()
            },
        }
        write_json(base_dir / "package.json", base_manifest)
        shutil.copyfile(LICENSE, base_dir / "LICENSE")
        base_bin = base_dir / "bin" / "perfect-doc.js"
        base_bin.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(LAUNCHER, base_bin)
        base_bin.chmod(0o755)

        platform_manifest = package_json(target, version, args.base_name, license_name)
        write_json(platform_dir / "package.json", platform_manifest)
        shutil.copyfile(LICENSE, platform_dir / "LICENSE")
        platform_binary = platform_dir / "bin" / target.binary_name
        platform_binary.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(binary, platform_binary)
        if target.os_name != "win32":
            platform_binary.chmod(0o755)

        npm = shutil.which("npm")
        if npm is None:
            raise PackageError("npm is required to create package tarballs")
        tarballs = stage / "tarballs"
        tarballs.mkdir(parents=True, exist_ok=True)
        npm_cache = stage / ".npm-cache"
        npm_cache.mkdir(parents=True, exist_ok=True)
        for directory in (base_dir, platform_dir):
            subprocess.run(
                [npm, "pack", "--cache", str(npm_cache), "--pack-destination", str(tarballs)],
                cwd=directory,
                check=True,
            )
        print(f"Staged private npm packages in {stage}")
        print(f"Built platform package {platform_name}@{version}")
        return 0
    except PackageError as error:
        print(f"package-npm: {error}", file=sys.stderr)
        return 2
    except (OSError, subprocess.CalledProcessError) as error:
        print(f"package-npm: package staging failed: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
