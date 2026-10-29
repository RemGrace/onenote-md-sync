# GitHub Actions: CI + Cross-Platform Release Builds

## Overview

Add GitHub Actions workflows for continuous integration and automated cross-platform binary releases for `any2md`.

## Workflows

### 1. CI (`ci.yml`)

**Trigger:** Push and PR to `master`
**Runner:** `ubuntu-latest`

Steps:
1. Checkout code
2. Install system deps: `cmake`, `libasound2-dev`
3. Cache `~/.cargo` and `target/`
4. `cargo fmt -- --check`
5. `cargo clippy -- -W clippy::all`
6. `cargo test`

### 2. Release (`release.yml`)

**Trigger:** Tag push matching `v*`

#### Build matrix

| Runner | Target | Notes |
|--------|--------|-------|
| `ubuntu-latest` | `x86_64-unknown-linux-gnu` | Needs cmake + libasound2-dev |
| `macos-latest` | `aarch64-apple-darwin` | Apple Silicon |
| `macos-13` | `x86_64-apple-darwin` | Intel Mac |
| `windows-latest` | `x86_64-pc-windows-msvc` | Needs cmake |

Each job: checkout, install Rust toolchain + deps, `cargo build --release --target <target>`, upload artifact.

#### Universal macOS binary

Depends on both macOS build jobs. Uses `lipo -create` to combine arm64 + x86_64 into a single universal binary.

#### Release job

Depends on all build + universal jobs:
1. Download all artifacts
2. Package into archives:
   - `any2md-v{tag}-linux-x86_64.tar.gz`
   - `any2md-v{tag}-macos-universal.tar.gz`
   - `any2md-v{tag}-windows-x86_64.zip`
3. Create GitHub Release with auto-generated notes, attach archives

## Artifact naming

Format: `any2md-{tag}-{os}-{arch}.{ext}`

- `.tar.gz` for macOS and Linux
- `.zip` for Windows

## System dependencies

- `cmake` required on all platforms (whisper-rs build)
- `libasound2-dev` required on Linux (cpal/ALSA)

## README update

Add release instructions:
- How to tag and push a release
- How to download and install on each platform

## Decisions

- CI runs on Linux only (fast, cheap, catches most issues)
- macOS universal binary instead of shipping two separate binaries
- Auto-generated release notes (no manual drafting)
- cmake installed on all runners (no feature flag gating for audio)
