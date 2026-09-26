nightly := "nightly-2026-06-16"
wild-version := "0.10.0"
wgsl-test-version := "0.2.35"
cargo-deny-version := "0.20.2"
cargo-about-version := "0.9.2"
reuse-version := "6.2.0"

version := `cargo metadata --format-version 1 --no-deps | jq -r '.packages[] | select(.name == "lapiz_app") | .version'`
os-name := os()
python := if os-name == "windows" { "python" } else { "python3" }

default:
    @just --list

setup: setup-base setup-android

setup-base: setup-rust setup-node setup-format setup-package setup-deny setup-reuse setup-linux

setup-ci: setup-base setup-ci-vulkan

setup-android:
    #!/usr/bin/env bash
    set -euo pipefail
    source android/toolchain.properties
    cli="android"
    if [ "{{ os-name }}" = "windows" ]; then cli="android.exe"; fi
    if ! command -v "$cli" >/dev/null; then
        echo "Install Android CLI and add it to PATH" >&2
        exit 1
    fi
    # Android is not exiting with 0 even on success
    "$cli" --no-metrics sdk install "platforms/android-$ANDROID_PLATFORM" "build-tools/$ANDROID_BUILD_TOOLS" "ndk/$ANDROID_NDK" || true
    rustup target add aarch64-linux-android x86_64-linux-android
    if [ "$(cargo ndk --version 2>/dev/null || true)" != "cargo-ndk $ANDROID_CARGO_NDK" ]; then
        RUSTFLAGS="" cargo install cargo-ndk --locked --version "$ANDROID_CARGO_NDK"
    fi

setup-rust:
    cargo --version

setup-node:
    #!/usr/bin/env bash
    set -euo pipefail
    expected="$(tr -d '[:space:]' < .node-version)"
    actual="$(node --version)"
    case "$actual" in
        "v$expected".*) ;;
        *) echo "Node.js $expected is required, found $actual" >&2; exit 1 ;;
    esac
    npm --version

setup-format:
    rustup toolchain install {{ nightly }} --profile minimal --component rustfmt --no-self-update

setup-package:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "$(cargo about --version 2>/dev/null || true)" != "cargo-about {{ cargo-about-version }}" ]; then
        RUSTFLAGS="" cargo install cargo-about --locked --version {{ cargo-about-version }} --features cli
    fi
    cargo about --version

setup-deny:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "$(cargo deny --version 2>/dev/null || true)" != "cargo-deny {{ cargo-deny-version }}" ]; then
        RUSTFLAGS="" cargo install cargo-deny --locked --version {{ cargo-deny-version }}
    fi

setup-reuse:
    #!/usr/bin/env bash
    set -euo pipefail
    if ! reuse --version 2>/dev/null | grep -Fq "{{ reuse-version }}"; then
        pipx install --force "reuse[charset-normalizer]=={{ reuse-version }}"
    fi

setup-linux: setup-linux-dependencies setup-linux-linker

setup-linux-dependencies:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "{{ os-name }}" != "linux" ]; then
        echo "setup-linux-dependencies: skipped on {{ os-name }}"
        exit 0
    fi
    sudo apt-get update
    sudo apt-get install -y --no-install-recommends \
        binutils clang pkg-config libx11-dev libxkbcommon-dev libxkbcommon-x11-dev \
        libwayland-dev libxcb1-dev libxcb-render0-dev libxcb-shape0-dev \
        libxcb-xfixes0-dev libfontconfig1-dev libudev-dev libdbus-1-dev \
        libasound2-dev libegl1-mesa-dev libgbm-dev libdrm-dev

setup-linux-linker: setup-linux-dependencies
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "{{ os-name }}" != "linux" ]; then
        echo "setup-linux-linker: skipped on {{ os-name }}"
        exit 0
    fi
    if ! wild --version 2>/dev/null | grep -Fq "{{ wild-version }}"; then
        RUSTFLAGS="" cargo install wild-linker --locked --version {{ wild-version }}
    fi
    wild --version

setup-ci-vulkan: setup-linux-dependencies
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "{{ os-name }}" != "linux" ]; then
        echo "setup-ci-vulkan: skipped on {{ os-name }}"
        exit 0
    fi
    # GitHub-hosted Linux runners need a software Vulkan implementation for WGSL tests.
    sudo apt-get install -y --no-install-recommends libvulkan1 mesa-vulkan-drivers

setup-for-fmt: setup-format

fmt:
    cargo +{{ nightly }} fmt --all
    tombi format

setup-for-test: setup-rust setup-node setup-linux

test: test-unit test-doc test-wgsl

test-unit:
    cargo test --workspace --all-targets --locked

test-doc:
    cargo test --workspace --doc --locked

test-wgsl:
    npx --yes wgsl-test@{{ wgsl-test-version }} run --projectDir crates/lapiz_color

setup-for-check: setup-rust setup-format setup-deny setup-reuse setup-linux

check platform="desktop" arch="": check-fmt (check-clippy platform arch) check-import-alias check-let-type-annotation check-deny check-reuse

check-fmt:
    cargo +{{ nightly }} fmt --all -- --check
    tombi lint

check-clippy platform="desktop" arch="":
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{ platform }}" in
        desktop)
            if [ -n "{{ arch }}" ]; then
                echo "desktop does not accept an architecture" >&2
                exit 2
            fi
            cargo clippy --workspace --exclude iced_winit --no-deps --all-targets --locked -- -D warnings
            ;;
        android)
            case "{{ arch }}" in
                aarch64) abi=arm64-v8a ;;
                x86_64) abi=x86_64 ;;
                *) echo "Android architecture must be aarch64 or x86_64" >&2; exit 2 ;;
            esac
            cargo ndk -P 28 -t "$abi" clippy --workspace --exclude iced_winit --exclude xtask --lib --no-deps --locked -- -D warnings
            ;;
        *) echo "check-clippy platform must be desktop or android" >&2; exit 2 ;;
    esac

# TODO: Remove this when clippy supports.
check-import-alias:
    cargo run --locked --quiet -p xtask -- import-alias-check --exclude iced_winit

# TODO: Remove this when clippy supports;
check-let-type-annotation:
    cargo run --locked --quiet -p xtask -- let-type-annotation-check --exclude iced_winit

check-deny:
    cargo deny check advisories bans licenses sources

check-reuse:
    reuse lint

setup-for-build: setup-rust setup-linux

build platform profile arch="":
    {{ python }} -m scripts.build "{{ platform }}" "{{ profile }}" "{{ arch }}"

run profile:
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{ profile }}" in
        dev) cargo run --locked ;;
        dev-local) cargo run --locked --features lapiz_dirs/dev_local ;;
        release) cargo run --release --locked ;;
        *) echo "run profile must be dev, dev-local, or release" >&2; exit 2 ;;
    esac

setup-for-package: setup-for-build setup-package

package platform profile arch="":
    {{ python }} -m scripts.package "{{ platform }}" "{{ profile }}" "{{ arch }}"

sync-iced-winit ref="HEAD":
    #!/usr/bin/env bash
    set -euo pipefail
    upstream="https://github.com/443eb9/iced.git"
    base="$(tr -d '[:space:]' < vendor/iced_winit/.upstream-rev)"
    git fetch --no-tags "$upstream" "{{ ref }}"
    next="$(git rev-parse 'FETCH_HEAD^{commit}')"
    if [ "$base" = "$next" ]; then exit 0; fi
    if ! git cat-file -e "$base^{commit}" 2>/dev/null; then
        git fetch --no-tags "$upstream" "$base"
    fi
    git diff --binary "$base" "$next" -- winit | git apply --3way -p2 --directory=vendor/iced_winit
    printf '%s\n' "$next" > vendor/iced_winit/.upstream-rev

verify-release-tag tag:
    #!/usr/bin/env bash
    set -euo pipefail
    expected="v{{ version }}"
    if [ "{{ tag }}" != "$expected" ]; then
        echo "tag {{ tag }} does not match lapiz_app version $expected" >&2
        exit 1
    fi
