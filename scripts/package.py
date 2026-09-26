"""Package built desktop and Android artifacts."""

import argparse
import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
import tarfile
import zipfile
from collections.abc import Iterable
from pathlib import Path
from typing import TypedDict

from . import android, build

REPO = Path(__file__).resolve().parent.parent
OUTPUT = REPO / "target/package"


class CargoPackage(TypedDict):
    name: str
    version: str


class CargoMetadata(TypedDict):
    packages: list[CargoPackage]


def output_of(*command: str) -> str:
    return subprocess.check_output(command, cwd=REPO, text=True).strip()


def compute_checksum_of(files: Iterable[Path], destination: Path) -> None:
    with destination.open("w") as output_file:
        for file in files:
            digest = hashlib.sha256()
            with file.open("rb") as input_file:
                for chunk in iter(lambda: input_file.read(1024 * 1024), b""):
                    digest.update(chunk)
            output_file.write(f"{digest.hexdigest()}  {file.name}\n")


def get_app_version() -> str:
    metadata: CargoMetadata = json.loads(
        output_of("cargo", "metadata", "--format-version", "1", "--no-deps")
    )
    return next(
        package["version"]
        for package in metadata["packages"]
        if package["name"] == "lapiz_app"
    )


def artifact_name_for(profile: str, system: str, arch: str) -> str:
    suffix = "-dev" if profile == "dev" else ""
    sha = output_of("git", "rev-parse", "HEAD")[:7]
    return f"lapiz-{get_app_version()}{suffix}-{sha}-{system}-{arch}"


def package_desktop(profile: str) -> None:
    system = (
        "windows"
        if sys.platform == "win32"
        else "macos"
        if sys.platform == "darwin"
        else "linux"
        if sys.platform.startswith("linux")
        else None
    )
    if not system:
        raise RuntimeError(f"unsupported packaging platform: {sys.platform}")

    machine = platform.machine().lower()
    arch = (
        "x86_64"
        if machine in ("x86_64", "amd64")
        else "arm64"
        if machine in ("aarch64", "arm64")
        else machine
    )
    name = artifact_name_for(profile, system, arch)
    if not name.startswith("lapiz-") or "/" in name or "\\" in name or ".." in name:
        raise RuntimeError(f"unsafe staging path: {name}")

    executable = build.build_desktop(profile)
    bindir = executable.parent
    binary = executable.name
    extension = "zip" if system == "windows" else "tar.gz"

    staging = OUTPUT / name
    archive = OUTPUT / f"{name}.{extension}"
    digest = OUTPUT / f"{name}.sha256"

    OUTPUT.mkdir(parents=True, exist_ok=True)
    if staging.exists():
        shutil.rmtree(staging)
    staging.mkdir()

    # TODO: We should embed readme, licenses, assets into the binary,
    #       instead of an archive, to match the behavior on android.
    #       Currently, android only packs the binary, checksum and debug symbols.
    #       In the future, packages for all platforms should only include a
    #       single binary, checksum for the binary, and debug symbols.
    for source, target in (
        (executable, binary),
        (REPO / "README.md", "README.md"),
        (REPO / "LICENSE", "LICENSE"),
        (REPO / "LICENSES/MIT.txt", "MIT.txt"),
    ):
        shutil.copy2(source, staging / target)

    if system == "linux":
        symbols = OUTPUT / f"{name}.debug"
        subprocess.run(
            ["objcopy", "--only-keep-debug", str(staging / binary), str(symbols)],
            check=True,
        )
        subprocess.run(["strip", "--strip-debug", str(staging / binary)], check=True)
        subprocess.run(
            ["objcopy", f"--add-gnu-debuglink={symbols}", str(staging / binary)],
            check=True,
        )
    elif system == "macos":
        dsym = OUTPUT / f"{name}.dSYM"
        symbols = OUTPUT / f"{name}.dSYM.tar.gz"
        if dsym.exists():
            shutil.rmtree(dsym)
        symbols.unlink(missing_ok=True)
        subprocess.run(["dsymutil", str(staging / binary), "-o", str(dsym)], check=True)
        subprocess.run(["strip", "-S", str(staging / binary)], check=True)
        with tarfile.open(symbols, "w:gz") as package:
            package.add(dsym, arcname=dsym.name)
        shutil.rmtree(dsym)
    else:
        symbols = OUTPUT / f"{name}.pdb"
        pdb = bindir / "lapiz_app.pdb"
        if not pdb.is_file():
            raise RuntimeError(f"missing debug symbols: {pdb}")
        shutil.copy2(pdb, symbols)

    subprocess.run(
        [
            "cargo",
            "about",
            "generate",
            "about.hbs",
            "--output-file",
            str(staging / "THIRD_PARTY_LICENSES.html"),
        ],
        cwd=REPO,
        check=True,
    )

    tracked = subprocess.check_output(
        ["git", "ls-files", "-z", "--", "assets"], cwd=REPO
    ).split(b"\0")
    for raw in filter(None, tracked):
        source = os.fsdecode(raw)
        if (
            subprocess.run(
                ["git", "check-ignore", "--no-index", "-q", "--", source],
                cwd=REPO,
                check=False,
            ).returncode
            == 0
        ):
            print(f"Excluded ignored asset: {source}")
            continue
        destination = staging / source
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(REPO / source, destination)

    archive.unlink(missing_ok=True)
    digest.unlink(missing_ok=True)
    if system == "windows":
        with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as package:
            for path in staging.rglob("*"):
                package.write(path, arcname=path.relative_to(OUTPUT))
    else:
        with tarfile.open(archive, "w:gz") as package:
            package.add(staging, arcname=name)
    compute_checksum_of([archive], digest)
    print(f"Packaged: {archive} + {digest} + {symbols}")


def package_android(profile: str, arch: str) -> None:
    rust_target = android.rust_target_for_arch(arch)
    out_dir = "release" if profile == "release" else "debug"
    apk = build.build_android(profile, arch, strip_symbols=True)

    symbols = REPO / "target" / rust_target / out_dir / "liblapiz_app.so"
    name = artifact_name_for(profile, "android", arch)

    OUTPUT.mkdir(parents=True, exist_ok=True)
    archive = OUTPUT / f"{name}.apk"
    debug_symbols = OUTPUT / f"{name}-liblapiz_app.so"
    digest = OUTPUT / f"{name}.sha256"
    shutil.copy2(apk, archive)
    shutil.copy2(symbols, debug_symbols)
    compute_checksum_of([archive, debug_symbols], digest)
    print(f"Packaged: {archive} + {debug_symbols} + {digest}")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("platform", choices=("desktop", "android"))
    parser.add_argument("profile", choices=("dev", "release"))
    parser.add_argument("arch", nargs="?")
    args = parser.parse_args()

    if args.platform == "desktop":
        if args.arch:
            parser.error("desktop does not accept an architecture")
        package_desktop(args.profile)
    elif args.platform == "android":
        if not args.arch:
            parser.error("android requires an architecture: aarch64 or x86_64")
        package_android(args.profile, args.arch)
    else:
        parser.error("platform must be desktop or android")


if __name__ == "__main__":
    try:
        main()
    except (
        OSError,
        ValueError,
        RuntimeError,
        subprocess.CalledProcessError,
        StopIteration,
    ) as error:
        sys.exit(str(error))
