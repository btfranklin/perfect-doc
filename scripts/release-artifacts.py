#!/usr/bin/env python3
"""Build and check a native Perfect Doc release archive."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import platform
import stat
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import zipfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
PACKAGE_HELPER = ROOT / "scripts" / "package-npm.py"
LICENSE = ROOT / "LICENSE"
README = ROOT / "packages" / "native" / "README.md"
DEFAULT_NOTICES = ROOT / "dist" / "licenses" / "THIRD-PARTY-NOTICES.txt"
EXAMPLE_DOCS = ROOT / "examples" / "integration" / "docs"
SUPPORTED_TARGETS = {
    "aarch64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
}


class ReleaseError(Exception):
    """A release archive could not be built or checked."""


def package_metadata() -> tuple[str, str]:
    with (ROOT / "Cargo.toml").open("rb") as source:
        package = tomllib.load(source)["package"]
    return package["name"], package["version"]


def target_host() -> str | None:
    machine = platform.machine().lower()
    if sys.platform == "darwin":
        return {
            "arm64": "aarch64-apple-darwin",
            "aarch64": "aarch64-apple-darwin",
        }.get(machine)
    if sys.platform.startswith("linux") and machine in {"x86_64", "amd64"}:
        libc, _ = platform.libc_ver()
        if libc.lower() == "glibc":
            return "x86_64-unknown-linux-gnu"
        return None
    if sys.platform == "win32" and machine in {"amd64", "x86_64"}:
        return "x86_64-pc-windows-msvc"
    return None


def load_package_helper():
    spec = importlib.util.spec_from_file_location("perfect_doc_package_npm", PACKAGE_HELPER)
    if spec is None or spec.loader is None:
        raise ReleaseError("cannot load the native binary validation helper")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def checksum(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def add_tar_file(archive: tarfile.TarFile, path: Path, name: str, mode: int) -> None:
    info = archive.gettarinfo(str(path), arcname=name)
    info.mode = mode
    info.uid = 0
    info.gid = 0
    info.uname = ""
    info.gname = ""
    info.mtime = 0
    with path.open("rb") as source:
        archive.addfile(info, source)


def write_archive(
    archive_path: Path,
    target: str,
    binary: Path,
    package_name: str,
    notices: Path,
    root_name: str,
) -> None:
    binary_name = package_name + (".exe" if target == "x86_64-pc-windows-msvc" else "")
    if target == "x86_64-pc-windows-msvc":
        with zipfile.ZipFile(archive_path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
            for source, name in (
                (binary, f"{root_name}/{binary_name}"),
                (LICENSE, f"{root_name}/LICENSE"),
                (README, f"{root_name}/README.md"),
                (notices, f"{root_name}/THIRD-PARTY-NOTICES.txt"),
            ):
                info = zipfile.ZipInfo(name)
                info.compress_type = zipfile.ZIP_DEFLATED
                info.external_attr = (stat.S_IFREG | (0o755 if source == binary else 0o644)) << 16
                archive.writestr(info, source.read_bytes())
        return

    with tarfile.open(archive_path, "w:gz") as archive:
        add_tar_file(archive, binary, f"{root_name}/{binary_name}", 0o755)
        add_tar_file(archive, LICENSE, f"{root_name}/LICENSE", 0o644)
        add_tar_file(archive, README, f"{root_name}/README.md", 0o644)
        add_tar_file(archive, notices, f"{root_name}/THIRD-PARTY-NOTICES.txt", 0o644)


def extract_archive(archive_path: Path, destination: Path) -> Path:
    if archive_path.suffix == ".zip":
        with zipfile.ZipFile(archive_path) as archive:
            archive.extractall(destination)
    else:
        with tarfile.open(archive_path, "r:gz") as archive:
            archive.extractall(destination)
    roots = list(destination.iterdir())
    if len(roots) != 1 or not roots[0].is_dir():
        raise ReleaseError("archive must contain one root directory")
    return roots[0]


def check_extracted(binary: Path, package_name: str, version: str, cwd: Path) -> None:
    expected = f"{package_name} {version}"
    try:
        version_result = subprocess.run(
            [str(binary), "--version"],
            cwd=cwd,
            check=True,
            capture_output=True,
            text=True,
        )
        if version_result.stdout.strip() != expected:
            raise ReleaseError(
                f"extracted binary reports {version_result.stdout.strip()!r}; expected {expected!r}"
            )
        subprocess.run(
            [str(binary), "check", "--format", "json", "--offline", "--no-banner", str(EXAMPLE_DOCS)],
            cwd=cwd,
            check=True,
            capture_output=True,
            text=True,
        )
        subprocess.run(
            [str(binary), "check", "--format", "json", "--offline", "--no-banner", str(cwd)],
            cwd=cwd,
            check=True,
            capture_output=True,
            text=True,
        )
        with tempfile.TemporaryDirectory(prefix="perfect-doc-broken-link-") as temporary:
            fixture = Path(temporary)
            (fixture / "README.md").write_text(
                "# Release check\n\n[Missing](missing.md)\n", encoding="utf-8"
            )
            failure_result = subprocess.run(
                [str(binary), "check", "--format", "json", "--offline", "--no-banner", str(fixture)],
                cwd=cwd,
                capture_output=True,
                text=True,
            )
            if failure_result.returncode != 1:
                raise ReleaseError(
                    f"broken-link check returned {failure_result.returncode}; expected 1"
                )
            try:
                report = json.loads(failure_result.stdout)
            except json.JSONDecodeError as error:
                raise ReleaseError("broken-link check did not return valid JSON") from error
            diagnostic = next(
                (
                    item
                    for item in report.get("diagnostics", [])
                    if item.get("rule") == "link.exists"
                ),
                None,
            )
            if diagnostic is None or diagnostic.get("target") != "missing.md":
                raise ReleaseError("broken-link check did not report link.exists for missing.md")
            if not isinstance(diagnostic.get("help"), str) or not diagnostic["help"].strip():
                raise ReleaseError("broken-link diagnostic has no repair help")
    except OSError as error:
        raise ReleaseError(f"cannot run the extracted native binary: {error}") from error
    except subprocess.CalledProcessError as error:
        detail = (error.stderr or error.stdout or str(error)).strip()
        raise ReleaseError(f"extracted binary check failed: {detail}") from error


def build(args: argparse.Namespace) -> Path:
    if args.target not in SUPPORTED_TARGETS:
        raise ReleaseError(f"unsupported target: {args.target}")

    package_name, version = package_metadata()
    if args.tag is not None and args.tag != f"v{version}":
        raise ReleaseError(f"tag must be v{version}; got {args.tag}")

    binary = args.binary.resolve()
    if not binary.is_file():
        raise ReleaseError(f"release binary does not exist: {binary}")

    helper = load_package_helper()
    target_info = helper.TARGETS.get(args.target)
    if target_info is None:
        raise ReleaseError(f"no binary signature rules exist for target: {args.target}")
    try:
        helper.binary_matches(binary, target_info, False)
    except helper.PackageError as error:
        raise ReleaseError(str(error)) from error

    host = target_host()
    if host != args.target:
        raise ReleaseError(
            f"target {args.target} cannot run on this host ({host or 'unsupported host'}); "
            "build and check each archive on its native target runner"
        )

    notices = args.notices.resolve()
    if not notices.is_file() or notices.stat().st_size == 0:
        raise ReleaseError(f"third-party notices file is missing or empty: {notices}")

    args.out_dir.mkdir(parents=True, exist_ok=True)
    extension = ".zip" if args.target == "x86_64-pc-windows-msvc" else ".tar.gz"
    archive_name = f"{package_name}-{version}-{args.target}{extension}"
    archive_path = args.out_dir.resolve() / archive_name
    root_name = f"{package_name}-{version}-{args.target}"
    write_archive(archive_path, args.target, binary, package_name, notices, root_name)

    checksum_path = archive_path.with_name(archive_path.name + ".sha256")
    checksum_path.write_text(
        f"{checksum(archive_path)}  {archive_path.name}\n", encoding="ascii", newline="\n"
    )

    with tempfile.TemporaryDirectory(prefix="perfect-doc-release-") as temporary:
        extracted_root = extract_archive(archive_path, Path(temporary))
        extracted_binary = extracted_root / (
            package_name + (".exe" if extension == ".zip" else "")
        )
        if extension != ".zip" and not os.access(extracted_binary, os.X_OK):
            raise ReleaseError("extracted Unix binary is not executable")
        check_extracted(extracted_binary, package_name, version, extracted_root)

    print(f"Built and checked {archive_path}")
    print(f"SHA-256: {checksum_path}")
    return archive_path


def main() -> int:
    parser = argparse.ArgumentParser(description="Build and check a native release archive.")
    parser.add_argument("--target", required=True, help="Rust target triple")
    parser.add_argument("--binary", required=True, type=Path, help="built native executable")
    parser.add_argument(
        "--out-dir", type=Path, default=ROOT / "dist" / "release", help="output directory"
    )
    parser.add_argument(
        "--notices",
        type=Path,
        default=DEFAULT_NOTICES,
        help="third-party notices file; defaults to dist/licenses/THIRD-PARTY-NOTICES.txt",
    )
    parser.add_argument("--tag", help="release tag; must match v plus the Cargo package version")
    args = parser.parse_args()

    try:
        build(args)
        return 0
    except ReleaseError as error:
        print(f"release-artifacts: {error}", file=sys.stderr)
        return 2
    except (OSError, tarfile.TarError, zipfile.BadZipFile) as error:
        print(f"release-artifacts: archive build failed: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
