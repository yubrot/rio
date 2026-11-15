# PLAN

## 1. OS-integrated title bar and frame on Windows, with tabs

Today Windows offers either the native title bar (`decorations = "Enabled"`) or none at all (`"Disabled"`, which also loses mouse move and resize). Support a style where:

- The frame stays OS-integrated: DWM shadow, rounded corners, resize borders, Snap Layouts, native caption buttons behavior.
- The title bar area is part of the blurred, translucent window body, and the tabs (`navigation.mode = "Tab"`) are drawn there, seamlessly (like Windows Terminal, or the macOS unified title bar).

Notes:

- `rio-window` Windows `WM_NCCALCSIZE` currently either keeps the full non-client area or removes all of it; this needs a mode that removes only the caption, plus `WM_NCHITTEST` for drag regions, resize borders and caption buttons (`HTMAXBUTTON` for Snap Layouts).
- The macOS path (`Decorations::Transparent`, `macos-use-unified-titlebar`, `padding_top_from_config`, `island.rs`) is the reference for laying out the tab strip in the title bar area.
