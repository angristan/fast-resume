#!/usr/bin/env python3
"""Build standalone binary for fast-resume using PyInstaller."""

import platform
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).parent
SRC = ROOT / "src" / "fast_resume"
DIST = ROOT / "dist"


def get_platform_tag() -> str:
    """Return a platform tag like 'macos-arm64', 'macos-x86_64', 'linux-x86_64'."""
    system = platform.system().lower()
    machine = platform.machine().lower()

    if system == "darwin":
        system = "macos"

    # Normalize architecture names
    arch_map = {
        "x86_64": "x86_64",
        "amd64": "x86_64",
        "aarch64": "arm64",
        "arm64": "arm64",
    }
    arch = arch_map.get(machine, machine)

    return f"{system}-{arch}"


def get_version() -> str:
    """Read the version from pyproject.toml."""
    pyproject = ROOT / "pyproject.toml"
    for line in pyproject.read_text().splitlines():
        if line.startswith("version"):
            return line.split('"')[1]
    raise RuntimeError("Could not find version in pyproject.toml")


def build() -> None:
    version = get_version()
    platform_tag = get_platform_tag()
    archive_name = f"fast-resume-{version}-{platform_tag}"

    print(f"Building fast-resume {version} for {platform_tag}")

    # Clean previous builds
    build_dir = ROOT / "build"
    if build_dir.exists():
        shutil.rmtree(build_dir)
    if DIST.exists():
        shutil.rmtree(DIST)

    # Collect asset data files
    assets_dir = SRC / "assets"
    asset_datas = []
    for png in assets_dir.glob("*.png"):
        # PyInstaller --add-data format: source:dest_dir
        sep = ";" if sys.platform == "win32" else ":"
        asset_datas.extend(["--add-data", f"{png}{sep}fast_resume/assets"])

    # Build with PyInstaller
    cmd = [
        sys.executable,
        "-m",
        "PyInstaller",
        "--name",
        "fr",
        "--onedir",
        "--console",
        "--noconfirm",
        "--clean",
        # Copy package metadata so importlib.metadata.version() works
        "--copy-metadata",
        "fast-resume",
        # Rich dynamically imports unicode data modules by version string
        "--collect-submodules",
        "rich._unicode_data",
        *asset_datas,
        # Entry point (wrapper to avoid relative import issues)
        str(ROOT / "entry_point.py"),
    ]

    print(f"Running: {' '.join(cmd)}")
    subprocess.run(cmd, check=True)

    # Create the archive
    dist_dir = DIST / "fr"
    if not dist_dir.exists():
        raise RuntimeError(f"Expected output directory {dist_dir} not found")

    # Also create a 'fast-resume' symlink/copy inside the dir
    fr_binary = dist_dir / "fr"
    if sys.platform == "win32":
        fr_binary_exe = dist_dir / "fr.exe"
        if fr_binary_exe.exists():
            shutil.copy2(fr_binary_exe, dist_dir / "fast-resume.exe")
    else:
        if fr_binary.exists():
            fast_resume_link = dist_dir / "fast-resume"
            if not fast_resume_link.exists():
                fast_resume_link.symlink_to("fr")

    # Create tar.gz archive
    archive_path = DIST / archive_name
    print(f"Creating archive: {archive_path}.tar.gz")
    shutil.make_archive(str(archive_path), "gztar", str(DIST), "fr")

    final_archive = DIST / f"{archive_name}.tar.gz"
    print(
        f"Built: {final_archive} ({final_archive.stat().st_size / 1024 / 1024:.1f} MB)"
    )


if __name__ == "__main__":
    build()
