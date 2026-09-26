"""Android host architecture and path helpers."""

import platform
import subprocess
import sys
from pathlib import Path

HOST_ARCH: str | None = {
    "x86_64": "x86_64",
    "AMD64": "x86_64",
    "aarch64": "aarch64",
    "arm64": "aarch64",
}.get(platform.machine())


def parse_java_properties(props: str) -> dict[str, str]:
    values: dict[str, str] = {}
    for line in props.splitlines():
        line = line.strip()
        if not line or line.startswith(("#", "!")) or "=" not in line:
            continue
        key, value = line.split("=", 1)
        values[key.strip()] = value.strip()
    return values


def native_path(path: str | Path) -> Path:
    if sys.platform == "win32" and str(path).startswith("/"):
        return Path(
            subprocess.check_output(["cygpath", "-w", str(path)], text=True).strip()
        )
    return Path(path)


def abi_for_arch(arch: str) -> str:
    match arch:
        case "aarch64":
            return "arm64-v8a"
        case "x86_64":
            return "x86_64"
        case _:
            raise ValueError(
                f"Unsupported Android architecture: {arch} (choose aarch64 or x86_64)"
            )


def rust_target_for_arch(arch: str) -> str:
    abi_for_arch(arch)
    return f"{arch}-linux-android"
