#!/usr/bin/env python3

import argparse
import os
from pathlib import Path, PurePosixPath
import shutil
import zipfile


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Extract the fast-resume executable from one built wheel."
    )
    parser.add_argument("--wheel-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    wheels = sorted(args.wheel_dir.glob("*.whl"))
    if len(wheels) != 1:
        raise RuntimeError(f"expected one wheel in {args.wheel_dir}, found {len(wheels)}")

    with zipfile.ZipFile(wheels[0]) as wheel:
        candidates = []
        for name in wheel.namelist():
            path = PurePosixPath(name)
            if len(path.parts) >= 3 and path.parts[-2] == "scripts" and path.name in {
                "fr",
                "fr.exe",
            }:
                candidates.append(name)

        if len(candidates) != 1:
            raise RuntimeError(
                f"expected one fr executable in {wheels[0]}, found {candidates}"
            )

        executable = PurePosixPath(candidates[0]).name
        destination = args.output / "bin" / executable
        destination.parent.mkdir(parents=True, exist_ok=True)
        with wheel.open(candidates[0]) as source, destination.open("wb") as target:
            shutil.copyfileobj(source, target)

    if os.name != "nt":
        destination.chmod(0o755)

    print(destination)


if __name__ == "__main__":
    main()
