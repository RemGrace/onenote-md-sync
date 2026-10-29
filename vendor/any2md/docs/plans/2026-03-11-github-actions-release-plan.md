# GitHub Actions CI + Release Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add GitHub Actions workflows for CI (lint/test on push) and cross-platform release builds (macOS universal, Linux x86_64, Windows x86_64) triggered by version tags.

**Architecture:** Two separate workflow files — `ci.yml` for fast feedback on every push/PR, `release.yml` for building and publishing binaries on tag push. Release workflow uses a build matrix + a universal binary merge step + a packaging/release step.

**Tech Stack:** GitHub Actions, Rust toolchain, cmake, lipo (macOS), tar/zip

---

### Task 1: Create CI workflow

**Files:**
- Create: `.github/workflows/ci.yml`

**Step 1: Create the CI workflow file**

```yaml
name: CI

on:
  push:
    branches: [master]
  pull_request:
    branches: [master]

env:
  CARGO_TERM_COLOR: always

jobs:
  check:
    name: Lint & Test
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install system dependencies
        run: sudo apt-get update && sudo apt-get install -y cmake libasound2-dev

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy

      - name: Cache cargo registry & build
        uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
            target
          key: ${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}
          restore-keys: ${{ runner.os }}-cargo-

      - name: Check formatting
        run: cargo fmt -- --check

      - name: Clippy
        run: cargo clippy -- -W clippy::all

      - name: Run tests
        run: cargo test
```

**Step 2: Verify the file is valid YAML**

Run: `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))"`
Expected: No output (valid YAML)

**Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: add CI workflow for lint and test on push/PR"
```

---

### Task 2: Create release workflow — build matrix

**Files:**
- Create: `.github/workflows/release.yml`

**Step 1: Create the release workflow with build jobs**

```yaml
name: Release

on:
  push:
    tags: ["v*"]

env:
  CARGO_TERM_COLOR: always

permissions:
  contents: write

jobs:
  build:
    name: Build ${{ matrix.target }}
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        include:
          - target: x86_64-unknown-linux-gnu
            os: ubuntu-latest
            artifact: any2md-linux-x86_64
          - target: aarch64-apple-darwin
            os: macos-latest
            artifact: any2md-macos-aarch64
          - target: x86_64-apple-darwin
            os: macos-13
            artifact: any2md-macos-x86_64
          - target: x86_64-pc-windows-msvc
            os: windows-latest
            artifact: any2md-windows-x86_64

    steps:
      - uses: actions/checkout@v4

      - name: Install system dependencies (Linux)
        if: runner.os == 'Linux'
        run: sudo apt-get update && sudo apt-get install -y cmake libasound2-dev

      - name: Install system dependencies (macOS)
        if: runner.os == 'macOS'
        run: brew install cmake

      - name: Install system dependencies (Windows)
        if: runner.os == 'Windows'
        run: choco install cmake --installargs 'ADD_CMAKE_TO_PATH=System' -y

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}

      - name: Build release binary
        run: cargo build --release --target ${{ matrix.target }}

      - name: Upload artifact (Unix)
        if: runner.os != 'Windows'
        uses: actions/upload-artifact@v4
        with:
          name: ${{ matrix.artifact }}
          path: target/${{ matrix.target }}/release/any2md

      - name: Upload artifact (Windows)
        if: runner.os == 'Windows'
        uses: actions/upload-artifact@v4
        with:
          name: ${{ matrix.artifact }}
          path: target/${{ matrix.target }}/release/any2md.exe
```

**Step 2: Verify YAML is valid**

Run: `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml'))"`
Expected: No output (valid YAML)

**Step 3: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci: add release workflow build matrix for all platforms"
```

---

### Task 3: Add universal binary and packaging jobs to release workflow

**Files:**
- Modify: `.github/workflows/release.yml` (append after `build` job)

**Step 1: Add the universal-binary and release jobs**

Append these jobs after the `build` job in `release.yml`:

```yaml
  universal-binary:
    name: Create macOS Universal Binary
    needs: build
    runs-on: macos-latest
    steps:
      - name: Download macOS aarch64 binary
        uses: actions/download-artifact@v4
        with:
          name: any2md-macos-aarch64
          path: aarch64

      - name: Download macOS x86_64 binary
        uses: actions/download-artifact@v4
        with:
          name: any2md-macos-x86_64
          path: x86_64

      - name: Create universal binary
        run: |
          chmod +x aarch64/any2md x86_64/any2md
          lipo -create -output any2md aarch64/any2md x86_64/any2md
          file any2md

      - name: Upload universal binary
        uses: actions/upload-artifact@v4
        with:
          name: any2md-macos-universal
          path: any2md

  release:
    name: Create GitHub Release
    needs: [build, universal-binary]
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Get version from tag
        id: version
        run: echo "VERSION=${GITHUB_REF_NAME}" >> "$GITHUB_OUTPUT"

      - name: Download Linux binary
        uses: actions/download-artifact@v4
        with:
          name: any2md-linux-x86_64
          path: linux

      - name: Download macOS universal binary
        uses: actions/download-artifact@v4
        with:
          name: any2md-macos-universal
          path: macos

      - name: Download Windows binary
        uses: actions/download-artifact@v4
        with:
          name: any2md-windows-x86_64
          path: windows

      - name: Package archives
        run: |
          VERSION=${{ steps.version.outputs.VERSION }}

          # Linux tar.gz
          chmod +x linux/any2md
          tar -czf "any2md-${VERSION}-linux-x86_64.tar.gz" -C linux any2md

          # macOS tar.gz
          chmod +x macos/any2md
          tar -czf "any2md-${VERSION}-macos-universal.tar.gz" -C macos any2md

          # Windows zip
          cd windows && zip "../any2md-${VERSION}-windows-x86_64.zip" any2md.exe && cd ..

      - name: Create GitHub Release
        uses: softprops/action-gh-release@v2
        with:
          generate_release_notes: true
          files: |
            any2md-*.tar.gz
            any2md-*.zip
```

**Step 2: Verify YAML is valid**

Run: `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml'))"`
Expected: No output (valid YAML)

**Step 3: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci: add universal binary creation and GitHub Release packaging"
```

---

### Task 4: Update README with release and installation instructions

**Files:**
- Modify: `README.md`

**Step 1: Add release installation section after the existing Installation heading**

Insert after `## Installation` and before `### Prerequisites`, a new subsection:

```markdown
### Download Pre-built Binaries

Download the latest release from the [Releases page](https://github.com/aspect-build/any2md/releases/latest).

| Platform | File |
|----------|------|
| macOS (Apple Silicon & Intel) | `any2md-vX.Y.Z-macos-universal.tar.gz` |
| Linux x86_64 | `any2md-vX.Y.Z-linux-x86_64.tar.gz` |
| Windows x86_64 | `any2md-vX.Y.Z-windows-x86_64.zip` |

```bash
# macOS / Linux
tar xzf any2md-*.tar.gz
chmod +x any2md
sudo mv any2md /usr/local/bin/

# Windows (PowerShell)
Expand-Archive any2md-*.zip -DestinationPath .
# Move any2md.exe to a directory in your PATH
```

### Build from Source
```

Rename the existing "### Prerequisites" to stay under the new "### Build from Source" subsection.

**Step 2: Add a Releasing section at the bottom of README**

Before `## Dependencies`, add:

```markdown
## Releasing

To publish a new release:

```bash
# Tag with a version
git tag v0.2.0
git push origin v0.2.0
```

This triggers the release workflow which builds binaries for all platforms and creates a GitHub Release with the artifacts.
```

**Step 3: Verify README renders correctly**

Read through the updated README to make sure headings and formatting are correct.

**Step 4: Commit**

```bash
git add README.md
git commit -m "docs: add binary download and release instructions to README"
```

---

### Task 5: Test CI workflow locally (smoke test)

**Step 1: Run the same commands CI would run**

```bash
cargo fmt -- --check
cargo clippy -- -W clippy::all
cargo test
```

Expected: All three pass with no errors.

**Step 2: Commit all work if not already committed**

Ensure everything is committed and clean.
