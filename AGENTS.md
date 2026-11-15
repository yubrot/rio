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
- `mise run run:windows`: build and launch it with the bundled ConPTY
- `mise run install:windows`: build and install it into `%LOCALAPPDATA%\Programs\Rio`
- `mise run check:windows`: clippy for the Windows target (leave warnings and errors inherited from upstream as they are, to avoid conflicts)
  - It always fails on those, so check only the diagnostics in the files you changed

Rio for Windows reads `%LOCALAPPDATA%\rio\config.toml`.

macOS builds run on a Mac, delegating to upstream's `Makefile`: `build:macos` (universal `Rio.app`), `run:macos`, `install:macos` (into `/Applications`), `check:macos`.

## Workflow

- Commit directly to the `yubrot-fork` branch; no feature branches or PRs.
- Running `mise run install:windows` to try out a build is fine at any time.
