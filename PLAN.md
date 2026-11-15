# PLAN

## 1. Consistent colors under `force-theme`

`force-theme` only picks between `adaptive-colors`; the native window keeps following the system app mode. With Windows apps in light mode and `force-theme = "dark"`:

- The Acrylic backdrop (`blur = true`, `DWMSBT_TRANSIENTWINDOW`) gets a light tint, so lowering `opacity` washes the background out to white.
- The title bar and frame stay light.
- The title bar color matches the terminal background only after it changes via OSC, not at startup (upstream rio#1446).

Goal: the window chrome, the backdrop and the terminal colors all follow the effective theme (`force-theme`, or the system theme when unset), at startup and on `ToggleAppearanceTheme`.

Notes:

- `rio-window` already has `Window::set_theme` / `WindowAttributes::with_theme` (Windows: `dark_mode::try_theme`), but `frontends/rioterm` never calls them.
- The Windows title bar color is set in `application.rs` on `WindowUpdate::Background` only.

## 2. OS-integrated title bar and frame on Windows, with tabs

Today Windows offers either the native title bar (`decorations = "Enabled"`) or none at all (`"Disabled"`, which also loses mouse move and resize). Support a style where:

- The frame stays OS-integrated: DWM shadow, rounded corners, resize borders, Snap Layouts, native caption buttons behavior.
- The title bar area is part of the blurred, translucent window body, and the tabs (`navigation.mode = "Tab"`) are drawn there, seamlessly (like Windows Terminal, or the macOS unified title bar).

Notes:

- `rio-window` Windows `WM_NCCALCSIZE` currently either keeps the full non-client area or removes all of it; this needs a mode that removes only the caption, plus `WM_NCHITTEST` for drag regions, resize borders and caption buttons (`HTMAXBUTTON` for Snap Layouts).
- The macOS path (`Decorations::Transparent`, `macos-use-unified-titlebar`, `padding_top_from_config`, `island.rs`) is the reference for laying out the tab strip in the title bar area.

## 3. Exe icon in cross builds

`mise run build:windows` (cargo-xwin from WSL) produces a `rio.exe` without the icon resource Explorer and the taskbar show. `frontends/rioterm/build.rs` embeds it in `load_app_icon` behind `#[cfg(windows)]` (the build host, not the target), and `winres` is a `cfg(windows)` build-dependency; off Windows, `winres` would use MinGW's `windres`, whose output doesn't fit the MSVC target anyway.

Goal: the icon is embedded whenever the target is Windows, whatever the host.

Notes:

- Replace `winres` with `embed-resource`, which compiles `.rc` files with `llvm-rc` when cross-compiling to MSVC (`llvm-rc` ships with clang; it is in `/usr/sbin` here). A one-line `.rc` (`1 ICON "rio-logo.ico"`) referencing `src/router/resources/images/rio-logo.ico` is enough.
- Gate on `CARGO_CFG_TARGET_OS == "windows"` only, and make the build-dependency unconditional.
- Worth sending upstream: it lets non-Windows hosts build the Windows package.
