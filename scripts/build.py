"""Build desktop binaries or Android APKs."""

import argparse
import subprocess
import sys
from pathlib import Path

from . import android

REPO = Path(__file__).resolve().parent.parent


def build_desktop(profile: str) -> Path:
    command = ["cargo", "build"]
    if profile == "release":
        command.append("--release")
    if profile == "dev-local":
        command.extend(("--features", "lapiz_dirs/dev_local"))
    subprocess.run([*command, "--locked"], check=True, cwd=REPO)
    return REPO / "target" / ("release" if profile == "release" else "debug") / (
        "lapiz_app.exe" if sys.platform == "win32" else "lapiz_app"
    )


def build_android(profile: str, arch: str, *, strip_symbols: bool = False) -> Path:
    variant = "DevDebug" if profile == "dev" else "ProdRelease"
    apk = REPO / (
        "android/app/build/outputs/apk/dev/debug/app-dev-debug.apk"
        if profile == "dev"
        else "android/app/build/outputs/apk/prod/release/app-prod-release.apk"
    )
    apk.unlink(missing_ok=True)
    wrapper = (
        REPO / "android" / ("gradlew.bat" if sys.platform == "win32" else "gradlew")
    )
    command = [wrapper, f":app:assemble{variant}", f"-PandroidArch={arch}"]
    if strip_symbols:
        command.append("-PstripRustSymbols=true")
    subprocess.run([*command, "--console=plain"], cwd=REPO / "android", check=True)
    return apk


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("platform", choices=("desktop", "android"))
    parser.add_argument("profile", choices=("dev", "dev-local", "release"))
    parser.add_argument("arch", nargs="?")
    args = parser.parse_args()

    if args.platform == "desktop":
        if args.arch:
            parser.error("desktop does not accept an architecture")

        build_desktop(args.profile)
    elif args.platform == "android":
        arch = str(args.arch) if args.arch else android.HOST_ARCH
        if not arch:
            parser.error("unknown architecture")
        if args.profile == "dev-local":
            parser.error("Android profile must be dev or release")

        build_android(args.profile, arch)
    else:
        parser.error("platform must be desktop or android")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        sys.exit(str(error))
