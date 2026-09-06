# Readero

A quiet native reader for Ubuntu, built around comfortable reading and keeping your place. Open a local PDF, DRM-free reflowable EPUB, or Markdown document; choose **Scroll** or **Pages**; return to the passage later.

Readero 0.1.1 is a working personal preview. The implementation has undergone native integration and visual checks. The complete release qualification and everyday reading sessions in the [MVP PRD](docs/MVP_PRD.md) are still separate gates; see the [implementation and QA record](docs/IMPLEMENTATION.md).

## Install locally

The supported platform is **Ubuntu 26.04**, validated on **amd64 with Papers 50.2**. Builds use the machine's native libraries and are not portable binaries for arbitrary Linux releases. GNU Make, Python 3.11+, Git, and rustup must already be available. From an existing checkout, skip the first two commands:

```sh
git clone git@github.com:michalito/readero.git
cd readero
make help            # list all commands; also the default for plain `make`
make setup           # install Ubuntu dependencies (sudo), update stable Rust, check readiness
make install         # build current source and install for your user; no sudo
```

`make setup` runs `apt-get update`, installs build and packaging dependencies, and installs/updates stable Rust with rustfmt and Clippy through rustup. If a selected older toolchain overrides stable, use `RUSTUP_TOOLCHAIN=stable make install`. If dependencies and Rust are already installed, skip setup and run `make doctor` or `make install` directly. Setup does not download or execute a rustup installer.

Installation puts the executable in `~/.local/bin/readero`, and the desktop launcher, icon, license notices, and sample document under `~/.local/share`. Launch **Readero** from the application menu or run `~/.local/bin/readero examples/quiet-reading.md`. The launcher uses the installed absolute path, so it works even if `~/.local/bin` is absent from your shell's `PATH`. Desktop caches are refreshed when the relevant utilities are available; log out and in if your desktop does not discover the new launcher immediately. Open With integration is registered without changing your default reader.

Every `make install` asks Cargo to build the current source in release mode with `--locked` and the ordinary desktop features. Cargo reuses unchanged build work. No pre-existing binary or old `.deb` in `dist/` is selected, and a previous smoke build is replaced by an ordinary build. The executable is replaced atomically, allowing an open Readero session to continue; restart it to use the new version.

### Install the latest source

```sh
make update          # fast-forward the tracked branch, then rebuild and install
# `make upgrade` is an alias
```

“Latest” means the newest commit on the current branch's configured upstream, not the newest release tag. Updates require a clean checkout (including untracked files), an attached branch, and an upstream; local changes and divergent history are never reset, stashed, or rebased automatically. For a checkout without an upstream, use `make install` for its current code, or configure your source remote and tracking branch before `make update`. Network or build failures stop installation; a build failure after a successful pull leaves the source updated and the installed app intact.

Application updates preserve `Cargo.lock` and the deliberately pinned native bindings/renderer assets. They do not run `cargo update` or upgrade those components independently. Run `make deps` and `make toolchain` when you want to refresh Ubuntu build packages and stable Rust.

### Install locations and packages

```sh
make install PREFIX="$HOME/.local"                    # default user installation
make install PREFIX=/usr DESTDIR=/tmp/readero-stage  # build a staging tree without sudo
make package                                         # create dist/readero_<version>_<arch>.deb
make install-deb                                     # build, then apt-install that exact package (sudo)
make uninstall                                       # remove the default user installation
```

`PREFIX` and `DESTDIR` must be absolute paths. Staging does not refresh host desktop caches and its launcher points to the final prefix, not the staging directory. For another prefix, use the same `PREFIX` when uninstalling. Custom desktop data locations may need to be added to `XDG_DATA_DIRS`. For system-wide installation, prefer `make install-deb` over running the build with sudo. If switching from the user installation to a package, first run `make uninstall` to avoid a user launcher shadowing the packaged version. Remove a package with `sudo apt remove readero`.

Packages use the version from `Cargo.toml`, the host architecture from dpkg, and runtime dependencies computed by `dpkg-shlibdeps`; cross-packaging is not supported. `make install-deb` supports rebuilding and reinstalling the same app version. `make uninstall` removes only the app's installed files and preserves reading history and bookmarks. `make clean` removes Cargo build products, leaving the installed app, `dist/`, and reading data intact.

Override `CARGO_TARGET_DIR` or `DIST_DIR` to relocate build products or packages. `CARGO` and `PYTHON` accept executable paths (not shell command strings). Example: `make package LOCAL_DEPS=1 CARGO_TARGET_DIR=/tmp/readero-target DIST_DIR=/tmp/readero-dist`.

![Readero reading view](docs/screenshots/reading.png)

## Reading

- Open through the file picker, drag-and-drop, a file argument, or Open With.
- Return through Continue/recent documents, with per-document position and appearance.
- Use contents, whole-document search, bookmarks, and Back/Forward for intentional jumps.
- Adjust reflowable text, width, line spacing, and light/warm/dark reading palettes. PDFs retain their original page colors and offer zoom, fit, and rotation.
- Focus (`F9`) hides reading controls; fullscreen (`F11`) is independent. Escape brings controls back.
- Inspect an EPUB/Markdown illustration by clicking it or focusing it and pressing Enter. Escape closes the image first and returns focus. Code and tables have keyboard scrolling when oversized.

Useful shortcuts: `Ctrl+O` Open, `Ctrl+F` Search, `Ctrl+G` / `Ctrl+Shift+G` next/previous result, `Ctrl+D` Bookmark, `Ctrl+L` PDF page, `Alt+Left/Right` Back/Forward, `F8` sidebar. The menu includes a shortcut reference.

Progress and bookmarks live in `$XDG_DATA_HOME/readero/reading.sqlite3` (normally `~/.local/share/readero/reading.sqlite3`). `READERO_DATA_DIR` selects an alternate directory for testing. Close the app before backing up the database. Original documents are never edited or imported into a managed library. Remove from recents preserves bookmarks. Locate reconnects a moved file.

Notes, highlights, OCR, cloud sync, DRM, fixed-layout EPUB guarantees, PDF forms, and source editing are outside this preview. Markdown supports CommonMark, tables, task lists, strikethrough, footnotes, and heading fragments; raw HTML is displayed as text. Automatic remote content and publication scripts are disabled. Markdown sidecar resources stay within the source directory tree.

## Build

`make doctor` checks Rust against `Cargo.toml` (currently 1.98) and the native library minimums: GTK 4.14, libadwaita 1.5, WebKitGTK 2.42, Papers View 49, and SQLite 3.34.1. To build without installing, use `make build`; `make run` builds and opens the app. The equivalent manual setup is:

```sh
sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev \
  libwebkitgtk-6.0-dev libpapers-dev libsqlite3-dev
cargo build --locked --release
make package
```

For the current workspace, development packages were checksum-verified and extracted under `/tmp`; no system packages were installed. Reproduce that optional setup with:

```sh
make local-deps
make doctor LOCAL_DEPS=1
make install LOCAL_DEPS=1
```

`LOCAL_DEPS=1` uses `scripts/cargo-local`, a machine-specific wrapper for that temporary prefix and the cached Cargo registry at `/tmp/readero-research/cargo-home` (override with `READERO_CARGO_HOME`). It applies the same environment to preflight checks and Cargo. Existing matching Ubuntu runtime libraries, a compiler, Rust, Python, and package metadata are still required. Temporary files may disappear after reboot; rerun `make local-deps` if needed. Ordinary installations should use `make setup` instead. The native runtime remains the Ubuntu-provided one.

## Check the implementation

`make check` runs formatting, strict Clippy, Rust tests, and installer tests. `make test-core` runs Rust tests without the desktop development packages; SQLite development headers are still required. `make test-install` checks installation in temporary directories and update safeguards using temporary Git repositories. All Cargo targets support `LOCAL_DEPS=1`. The individual Rust checks are:

```sh
cargo fmt --package readero --check
cargo test --locked
cargo clippy --no-deps --all-targets --features smoke --locked -- -D warnings
```

Use `scripts/cargo-local` in place of `cargo` with the temporary development prefix. `--no-deps` keeps strict linting focused on authored code; vendored upstream bindings remain unchanged.

The opt-in `smoke` feature drives the **real native application** and captures local screenshots and assertions. It is excluded from the ordinary executable. Python Cairo, Xvfb, and `dbus-run-session` are required for the isolated display route:

```sh
python3 tests/fixtures/generate.py
cargo build --locked --release --features smoke
python3 scripts/native-smoke.py --file examples/quiet-reading.md \
  --output /tmp/readero-check-md
python3 scripts/native-smoke.py --file /tmp/readero-build/fixtures/reading.epub \
  --output /tmp/readero-check-epub --stress
cargo build --locked --release  # restore an ordinary build before packaging
```

The smoke checks include exact bookmark recovery after edits, delayed images, width changes, overlapping chapter loads and jumps, keyboard affordances, and PDF restoration at every rotation in both reading modes. Add `--edit` to exercise source watching, atomic replacement, related-file heading links, and Back/Forward using a temporary Markdown copy. Keep the smoke executable separate if running other Cargo builds concurrently; those builds can replace it with an ordinary executable.

For the protected-PDF flow, install the test-only Python `pypdf` dependency, then run `python3 tests/fixtures/protect.py /tmp/readero-build/fixtures/reading.pdf /tmp/readero-build/fixtures/protected.pdf` and pass `--protected-fixture` with that file.

Use a fresh output directory for each run. Evidence contains reading locators, passages, and screenshots: keep personal-document runs outside the repository. `checks.json` contains only safe boolean outcomes. `--wayland` exercises the current desktop; the default uses Xvfb/software rendering. These flows establish behavior, not PRD startup/frame-time percentiles.

Separate native probes use the optimized application without the smoke driver's fixed navigation waits:

```sh
python3 scripts/native-qualify.py --binary /path/to/smoke-enabled-readero \
  --file examples/quiet-reading.md --output /tmp/readero-restart --mode restart
python3 scripts/native-qualify.py --binary /path/to/smoke-enabled-readero \
  --file examples/quiet-reading.md --output /tmp/readero-timing --mode timing
xvfb-run -a dbus-run-session -- python3 scripts/accessibility-smoke.py \
  --binary /path/to/readero --output /tmp/readero-accessibility
```

`restart` checks forced termination, an interrupted SQLite writer, and closing during a layout change, all against isolated state. `timing` collects 10 launches and 30 reopens on the current Wayland desktop; `idle` measures the actual app process tree after 40 turns and 20 mode changes. These are diagnostic observations, not a claim that the full PRD performance protocol has passed. The accessibility script inspects native controls, headings, and text through AT-SPI; it does not run an Orca usability session. Python GI/AT-SPI bindings are required for that check.

## Code and decisions

Rust/GTK4/libadwaita own the shell; Papers owns PDF rendering; WebKitGTK with pinned foliate-js handles EPUB and rendered Markdown. A dedicated worker owns SQLite. Document identity and versioned locators are independent of the presentation mode, leaving a path to linked notes later.

- [Finalized MVP PRD](docs/MVP_PRD.md)
- [Stack decisions and sources](docs/STACK_DECISIONS.md)
- [Implementation and QA](docs/IMPLEMENTATION.md)
- [Original component research](docs/research/VALIDATION.md)
- [Upstream provenance and licenses](vendor/README.md)

This is a personal project. Upstream source and license notices are preserved in `vendor/` and `assets/foliate/`.
