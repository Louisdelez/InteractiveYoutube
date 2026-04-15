# Cross-platform plan — Koala TV desktop

Target platforms:

| Platform            | Target triple                | Stage   | Notes                                               |
| ------------------- | ---------------------------- | ------- | --------------------------------------------------- |
| Linux x86_64        | `x86_64-unknown-linux-gnu`   | ✅ done | Reference implementation, dual-mpv + X11 overlays   |
| Windows x86_64      | `x86_64-pc-windows-gnu/msvc` | 🟡 P1    | Compiles; mpv runs in a secondary window           |
| macOS x86_64 (Intel)| `x86_64-apple-darwin`        | 🟡 P1    | Compiles; mpv runs in a secondary window           |
| macOS aarch64 (M1+) | `aarch64-apple-darwin`       | 🟡 P1    | Same as Intel                                        |

Legend: ✅ shipping · 🟡 compiles, needs integration · 🔜 planned.

## Phase 1 — Compile everywhere (DONE)

Goal: the code **builds** on all four targets. Linux stays fully functional; other platforms launch but render mpv in a separate top-level window (no in-app embedding).

What's done:

- All X11-only modules gated with `#![cfg(target_os = "linux")]` or equivalent:
  - `views/backup_player.rs`
  - `views/popup_menu.rs`
  - `views/xft_text.rs`
  - `views/memory_cache.rs`
  - `views/channel_badge.rs`
  - `views/loading_overlay.rs`
- `views/tooltip.rs` has a Linux path and a no-op stub for other OSes.
- `views/player.rs` — 44+ `cfg(target_os = "linux")` gates. The non-Linux branch creates a minimal mpv instance without `wid`, so mpv opens its own OS window.
- `app.rs` — 10+ gates around sidebar badge updates, backup swaps, memory cache.
- `Cargo.toml`: `x11-dl` already scoped to `[target."cfg(target_os = \"linux\")".dependencies]`.
- CI matrix (`.github/workflows/desktop-ci.yml`) runs `cargo check` on `ubuntu-latest`, `macos-13` (Intel), `macos-14` (Apple Silicon), `windows-latest`.

Verified locally: `cargo check --target x86_64-pc-windows-gnu` succeeds (warnings only).

## Phase 2 — In-window mpv embed (TODO)

Replace the secondary-window mpv on Windows/macOS with a real embed inside the GPUI window, mirroring the Linux X11 experience.

libmpv accepts three `wid` formats:
- **Linux**: X11 `Window` (u64) — currently used
- **Windows**: `HWND` (passed as i64)
- **macOS**: `NSView*` pointer (passed as i64)

Implementation sketch:

1. Expose the native window handle from GPUI via [`raw-window-handle`](https://crates.io/crates/raw-window-handle) (already a dep).
2. In `player.rs`, extract the handle per platform:
   ```rust
   #[cfg(target_os = "windows")]
   let wid = match gpui_handle { RawWindowHandle::Win32(h) => h.hwnd.get() as i64, _ => panic!() };
   #[cfg(target_os = "macos")]
   let wid = match gpui_handle { RawWindowHandle::AppKit(h) => h.ns_view.as_ptr() as i64, _ => panic!() };
   ```
3. Pass `wid` to mpv init.
4. The GPUI window is a *parent*; mpv will render over it. On macOS the caveat is the content view's layer hierarchy — may need an explicit subview.

**Blocker**: this needs actual Windows and macOS machines to iterate. The GPUI upstream (Zed) tests on macOS + Linux; Windows is experimental. I don't have a Mac or Windows box to validate.

## Phase 3 — Overlay replacements (TODO)

On Linux the tooltip, popup menu, channel badge, loading overlay and backup player are all **native X11 sibling windows** — they have to be, because GPUI can't composite *above* the mpv GPU surface. On macOS/Windows we need equivalents.

Three options, from cheapest to best:

1. **GPUI-only** (easiest): move the overlays into the GPUI tree. Works only if mpv renders to a GPU texture (`vo=libmpv` + render callback) that we then paint inside GPUI. Requires a substantial player rewrite.

2. **Native platform windows** (matches Linux pattern): Cocoa (`NSWindow` override-redirect equivalent) on macOS, `CreateWindowExW` on Windows. One file per platform under `cfg(target_os)`. High fidelity but triples the code for these features.

3. **Disable advanced overlays off-Linux**: no dual-mpv "zero-cut" swap, no popup menu (use gpui-component `Popover` instead), no channel badge overlay (bake into mpv with lavfi?). Lower feature parity but quickest to ship.

Suggested path: option 3 for v1 Windows/macOS (fast to ship), option 2 for v2 if demand.

## Phase 4 — Distribution

### Linux (.deb / AppImage / tarball)

- libmpv dependency: declare `libmpv2` as a package dep, tell users to `apt install libmpv-dev` for dev or `libmpv2` for runtime.
- Packaging: `cargo-deb` for Debian/Ubuntu; `appimagetool` for AppImage.
- CI: `ubuntu-20.04` runner for older glibc compatibility.

### macOS (.dmg)

- **libmpv**: bundle `libmpv.dylib` inside the `.app/Contents/Frameworks/` so users don't need Homebrew. Copy from `brew --prefix mpv`.
- **Apple Silicon + Intel**: build two separate binaries then `lipo -create` into a universal binary, or ship two DMGs.
- **Code signing**: Developer ID certificate required for distribution. Apple notarization via `xcrun notarytool` in CI.
- **Bundle**: `cargo-bundle` (or Tauri Bundler) to produce the `.app`.
- **Entitlements**: network client, no sandbox for the first release.

### Windows (.msi / .exe)

- **libmpv**: bundle `libmpv-2.dll` + `mpv.dll` from the [mpv.io releases](https://mpv.io/installation/). Ship next to the exe.
- **Installer**: `cargo-wix` (MSI via WiX Toolset) or Tauri Bundler.
- **Code signing**: optional but recommended; SmartScreen warnings without it.
- **MSVC vs GNU**: MSVC is the canonical target for distribution (`x86_64-pc-windows-msvc`). GNU is only used for cross-compile from Linux dev boxes.

### Auto-update

On Linux the yt-dlp updater already writes to `$XDG_DATA_HOME/KoalaTV/bin/`. On Windows the equivalent is `%APPDATA%\KoalaTV\bin\`, on macOS `~/Library/Application Support/KoalaTV/bin/`. Need to swap the platform-specific paths in `services/ytdlp_updater.rs`.

## Per-platform build

### Linux

```bash
sudo apt install libmpv-dev libx11-dev
cd desktop
cargo run --release
```

### Windows

Prerequisites:
- Rust (stable, `x86_64-pc-windows-msvc` toolchain via `rustup default stable-msvc`).
- [libmpv Windows build](https://sourceforge.net/projects/mpv-player-windows/files/libmpv/) — extract `mpv-2.dll` and `libmpv.lib` into a folder, set `LIBMPV_MPV_SOURCE=<that folder>` or link via MPV_LIB_DIR env var (see libmpv2-rs README).

```cmd
cd desktop
set LIBMPV_MPV_SOURCE=C:\libmpv
cargo build --release --target x86_64-pc-windows-msvc
```

### macOS

```bash
brew install mpv
cd desktop
# Apple Silicon
cargo build --release --target aarch64-apple-darwin
# Intel
cargo build --release --target x86_64-apple-darwin
# Universal binary
lipo -create -output target/universal/koala-tv-desktop \
  target/aarch64-apple-darwin/release/koala-tv-desktop \
  target/x86_64-apple-darwin/release/koala-tv-desktop
```

## CI matrix

See `.github/workflows/desktop-ci.yml`. Runs on push + PR. Current state: `cargo check` on all 4 targets; caches the Cargo registry + index + build artifacts.

Future: `cargo build --release` + artifact upload + signed-release step on tag push.

## Known gaps

- Phase 2 and 3 require a Mac and a Windows machine for iteration. Without them, the code compiles but the non-Linux experience is "mpv pops a second window, GPUI window is mostly empty".
- GPUI Windows support is marked experimental upstream; expect friction.
- libmpv2-rs on Windows requires the libmpv2 headers + import library at build time. The CI job would need to set these up per runner (fetch the Windows libmpv release in the job).

## Checklist

- [x] Gate X11 code behind cfg.
- [x] Stub tooltip off-Linux.
- [x] Linux + Windows `cargo check` pass.
- [x] CI matrix with cargo check on all 4 targets.
- [x] This plan document.
- [ ] Windows runner sets up libmpv and `cargo check` succeeds there.
- [ ] macOS runners set up libmpv via brew and `cargo check` succeeds.
- [ ] Wire mpv `wid=HWND` on Windows.
- [ ] Wire mpv `wid=NSView*` on macOS.
- [ ] Replace X11 tooltip/menu/badge/loading with GPUI or native equivalents off-Linux.
- [ ] `cargo-bundle` / `cargo-wix` for distribution.
- [ ] Per-platform yt-dlp updater path.
- [ ] Code signing pipeline (macOS notarization, Windows optional).
