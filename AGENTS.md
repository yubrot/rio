# AGENTS.md

A personal fork of [Rio](https://github.com/raphamorim/rio) (`upstream`), mainly to improve it on Windows. Planned work is in [PLAN.md](PLAN.md).

## Layout

- `frontends/rioterm`: the app (`rio` binary)
- `rio-window`: windowing, a fork of winit (Windows: `src/platform_impl/windows`)
- `sugarloaf`: the renderer
- `rio-backend`: config and shared state
- `rio-vt`, `rio-grid`: terminal emulation
- `teletypewriter`: PTY (ConPTY on Windows)

## Development

Tools are managed by mise (`mise.toml`). Windows builds are cross-compiled from WSL:

- `mise run build:windows`: build `rio.exe`
- `mise run dev:windows`: build it in the `fast` profile and launch it with the bundled ConPTY
- `mise run install:windows`: build and install it into `%LOCALAPPDATA%\Programs\Rio`
- `mise run test:windows`: rioterm's tests for the Windows target (extra args go to `cargo test`)
- `mise run check:windows`: clippy for the Windows target
  - It always fails on lints inherited from upstream; fix the ones in files you change, leave the rest as they are
  - Errors in `rio-window` stop it before `rioterm` is checked; run `cargo xwin clippy --release -p rioterm --target x86_64-pc-windows-msvc` (without `-D warnings`) to see all diagnostics

The release profile (LTO, one codegen unit, no incremental) is for shipping and takes minutes per rebuild; build with the `fast` profile or test in the dev profile to iterate.

Rio for Windows reads `%LOCALAPPDATA%\rio\config.toml`.

macOS builds run on a Mac: `build:macos` (universal `Rio.app`) and `install:macos` (into `/Applications`) delegate to upstream's `Makefile`, along with `dev:macos`, `test:macos` and `check:macos`.

## Workflow

- Commit directly to the `yubrot-fork` branch; no feature branches or PRs.
- Running `mise run dev:windows` to try out a build is fine at any time; run `mise run install:windows` only when asked, since the installed Rio may be in use.
