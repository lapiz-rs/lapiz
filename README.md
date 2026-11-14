# Lapiz

_Project logo under construction_

[![Build Status](https://github.com/lapiz-rs/lapiz/actions/workflows/build.yml/badge.svg)](https://github.com/lapiz-rs/lapiz/actions/workflows/build.yml)
[![License: GPL-3.0-or-later](https://img.shields.io/badge/License-GPL--3.0--or--later-blue.svg)](LICENSE)
[![Platforms](https://img.shields.io/badge/Platform-Windows%20%7C%20Linux%20%7C%20macOS%20%7C%20Android-informational)](#features--highlights)
[![Built with Rust](https://img.shields.io/badge/Built%20with-Rust-dca282.svg)](https://www.rust-lang.org/)

> [!WARNING]
> This project is still in a pre-pre-pre-alpha stage and is **absolutely not** ready for production use. Expect plenty of bugs, missing features, and incomplete code!

![](./docs/readme/main.png)

> Cute orange photo by Dariusz Duchiewicz from [Pexels](https://www.pexels.com/photo/bright-basket-of-oranges-and-apples-36492525/)

A GPU-powered, programmable, highly customizable, and blazingly fast digital painting program written in Rust—built with ❤ and passion, and open-source forever under the GPL-3.0-or-later license.

The name **Lapiz** comes from *lápiz*, the Spanish word for "pencil". It also echoes *lapis lazuli*—the deep-blue metamorphic rock prized since antiquity as the source of genuine ultramarine, one of the oldest and most precious blue pigments in art history (which is also why our native project file format is `.lazuli`). As for the pronunciation: neither English nor Spanish is my native language, so feel free to pronounce it however you like.

## Features & Highlights

- **Node-Based Programmable Brushes & Filters**: Instead of locking you into a fixed set of sliders, Lapiz drives its brush engine, filters, and layer effects through a programmable shader node graph (powered by WESL/WGSL). Build custom brush behaviors and image processing pipelines visually.
- **End-to-End GPU-Accelerated Pipeline**: Built from the ground up on `wgpu`. Tile-based canvas rendering, stroke stamping, layer compositing, anti-aliasing, and filters are all executed on the GPU for ultra-low latency and smooth high-resolution painting.
- **Full Color Management Pipeline**: Accurate, consistent colors from canvas to screen without sacrificing speed. Lapiz compiles ICC color profiles directly into GPU shaders, delivering professional-grade color accuracy and perceptual color spaces (such as `OkLab` and `CIE Lab`) at full GPU speed.
- **Cross-Application Brush Ecosystem**: Features an independent, clean-room Adobe Photoshop (`.abr`) brush parser out of the box, with planned support for other major brush formats including Procreate (`.brushset`), Clip Studio Paint (`.sut`), and Krita (`.kpp`).
- **Crash-Resilient `.lazuli` Document Format**: Our native `.lazuli` format is a tile-based archive backed by SQLite, architected from day one with incremental saving in mind *(WIP)*—so an unexpected crash or power loss won't corrupt your entire painting.
- **Everywhere You Paint**: First-class cross-platform support across **Windows**, **Linux** (Wayland & X11), **macOS**, and **Android** from a single codebase.

## Development

Lapiz uses [just](https://just.systems/) to orchestrate all setup, build, test, and packaging tasks.

### Prerequisites

Make sure the following basic tools are installed and available in your `PATH`:

- [Rust](https://rustup.rs/) (toolchain version is pinned automatically via [`rust-toolchain.toml`](rust-toolchain.toml))
- [just](https://just.systems/) and [`jq`](https://jqlang.org/)
- `bash` (pre-installed on Linux/macOS; included with Git for Windows)
- *(Optional, for tests & linting)*: [Node.js 24](.node-version) (for WGSL shader tests), [`pipx`](https://pipx.pypa.io/) (for REUSE license checks), and [`tombi`](https://tombi-toml.github.io/tombi/) (for TOML formatting)

### Build & Run

For a quick desktop build and run:

```sh
just setup-for-build
just run dev-local
```

If you want to set up the full development environment:

- `just setup-base` — sets up all desktop development, formatting, and linting tools.
- `just setup` — sets up everything in `setup-base` **plus** the Android SDK/NDK toolchain (requires the Android CLI in `PATH`).

That's it! Run `just` to list all available recipes, or `just --usage <recipe>` for detailed parameters of a specific command (such as `just test`, `just check`, or `just build`).

## Contribute

Contributions are welcome through GitHub [Issues](https://github.com/lapiz-rs/lapiz/issues) and [Pull Requests](https://github.com/lapiz-rs/lapiz/pulls).

### LLM Policy

We accept LLM-assisted contributions, provided you follow these ground rules:

- **Disclose LLM usage** in your PR or issue.
- **Review everything yourself** before submitting. You are fully responsible for understanding and verifying any LLM-generated code or text. Please do not copy-paste review feedback into an LLM and paste its raw response straight back—we want to hear **your** thoughts, not an LLM's.
- **Leave `good first issue` tasks for humans.** Please do not rely on an LLM to complete `good first issue` tickets, even if you review the output and are confident in its quality. Those issues exist to help human contributors get started with the codebase. :)

## Special Thanks

- [Bevy](https://bevy.org/) for being an awesome game engine and providing an elegant reference for building computer graphics architectures on top of `wgpu`.
- [Blender](https://www.blender.org/) for inspiring me to use node graphs to build a programmable brush and filter engine.
- [Krita](https://krita.org/) for setting the standard as an excellent open-source digital painting application. The gap-closing algorithm in our bucket fill tool is especially inspired by Krita.

## License

Lapiz is licensed under the **GPL-3.0-or-later** license. Certain crates, vendored components, and assets are distributed under other permissive licenses (such as `MIT`, `Apache-2.0`, and the Pexels license). See [`REUSE.toml`](REUSE.toml) for full per-path copyright and license details.
