nightly := "nightly-2026-06-16"
wild-version := "0.10.0"
wgsl-test-version := "0.2.35"
cargo-deny-version := "0.20.2"
cargo-about-version := "0.9.2"
reuse-version := "6.2.0"

version := `cargo metadata --format-version 1 --no-deps | jq -r '.packages[] | select(.name == "lapiz_app") | .version'`
sha := `git rev-parse HEAD | cut -c1-7`
target := `rustc -vV | sed -n 's/^host: //p'`
os-name := os()

default:
    @just --list

setup: setup-rust setup-node setup-format setup-package setup-deny setup-reuse setup-linux

setup-ci: setup setup-ci-vulkan

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

check: check-fmt check-clippy check-deny check-reuse

check-fmt:
    cargo +{{ nightly }} fmt --all -- --check
    tombi lint

check-clippy:
    cargo clippy --workspace --all-targets --locked -- -D warnings

check-deny:
    cargo deny check advisories bans licenses sources

check-reuse:
    reuse lint

setup-for-build: setup-rust setup-linux

build profile:
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{ profile }}" in
        dev) cargo build --locked ;;
        release) cargo build --release --locked ;;
        *) echo "profile must be dev or release" >&2; exit 2 ;;
    esac

run profile:
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{ profile }}" in
        dev) cargo run --locked ;;
        dev-local) cargo run --locked --features lapiz_dirs/dev_local ;;
        release) cargo run --release --locked ;;
        *) echo "profile must be dev, dev-local or release" >&2; exit 2 ;;
    esac

verify-release-tag tag:
    #!/usr/bin/env bash
    set -euo pipefail
    expected="v{{ version }}"
    if [ "{{ tag }}" != "$expected" ]; then
        echo "tag {{ tag }} does not match lapiz_app version $expected" >&2
        exit 1
    fi

setup-for-package: setup-for-build setup-package

package profile: (build profile)
    #!/usr/bin/env bash
    set -euo pipefail

    case "{{ profile }}" in
        dev)
            bindir=target/debug
            ver="{{ version }}-dev"
            ;;
        release)
            bindir=target/release
            ver="{{ version }}"
            ;;
        *)
            echo "profile must be dev or release" >&2
            exit 2
            ;;
    esac

    host_target="{{ target }}"
    case "$host_target" in
        x86_64-*) arch=x86_64 ;;
        aarch64-*) arch=arm64 ;;
        *) arch="${host_target%%-*}" ;;
    esac

    if [ "{{ os-name }}" = "windows" ]; then
        binary=lapiz_app.exe
        ext=zip
    else
        binary=lapiz_app
        ext=tar.gz
    fi

    name="lapiz-$ver-{{ sha }}-{{ os-name }}-$arch"
    staging="target/package/$name"
    archive="target/package/$name.$ext"
    checksum="target/package/$name.sha256"
    third_party="$staging/THIRD_PARTY_LICENSES.html"

    case "$staging" in
        target/package/lapiz-*) ;;
        *) echo "unsafe staging path: $staging" >&2; exit 1 ;;
    esac

    mkdir -p target/package
    if [ -e "$staging" ]; then
        find "$staging" -depth -delete
    fi
    mkdir -p "$staging"

    cp "$bindir/$binary" "$staging/"
    cp README.md LICENSE "$staging/"
    cp LICENSES/MIT.txt "$staging/MIT.txt"

    case "{{ os-name }}" in
        linux)
            debug_symbols="target/package/$name.debug"
            objcopy --only-keep-debug "$staging/$binary" "$debug_symbols"
            strip --strip-debug "$staging/$binary"
            objcopy --add-gnu-debuglink="$debug_symbols" "$staging/$binary"
            ;;
        macos)
            dsym="target/package/$name.dSYM"
            debug_symbols="$dsym.tar.gz"
            rm -rf "$dsym" "$debug_symbols"
            dsymutil "$staging/$binary" -o "$dsym"
            strip -S "$staging/$binary"
            tar -C target/package -czf "$debug_symbols" "$name.dSYM"
            rm -rf "$dsym"
            ;;
        windows)
            debug_symbols="target/package/$name.pdb"
            if [ ! -f "$bindir/lapiz_app.pdb" ]; then
                echo "missing debug symbols: $bindir/lapiz_app.pdb" >&2
                exit 1
            fi
            cp "$bindir/lapiz_app.pdb" "$debug_symbols"
            ;;
        *)
            echo "unsupported packaging platform: {{ os-name }}" >&2
            exit 1
            ;;
    esac

    cargo about generate about.hbs --output-file "$third_party"

    # Include only tracked assets that are not matched by .gitignore.
    # During local development, we may introduce some external assets for testing
    # like bundles created by someone else. They should not be included in the
    # packaged output.
    while IFS= read -r -d '' source; do
        if git check-ignore --no-index -q -- "$source"; then
            echo "Excluded ignored asset: $source"
            continue
        fi
        destination="$staging/$source"
        mkdir -p "$(dirname "$destination")"
        cp "$source" "$destination"
    done < <(git ls-files -z -- assets)

    rm -f "$archive" "$checksum"
    if [ "$ext" = zip ]; then
        /c/Windows/System32/tar.exe -C target/package -caf "$archive" "$name"
    else
        tar -C target/package -czf "$archive" "$name"
    fi

    if command -v sha256sum >/dev/null; then
        (cd target/package && sha256sum "$name.$ext" > "$name.sha256")
    else
        (cd target/package && shasum -a 256 "$name.$ext" > "$name.sha256")
    fi

    echo "Packaged: $archive + $checksum + $debug_symbols"
