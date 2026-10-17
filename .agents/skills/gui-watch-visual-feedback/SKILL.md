---
name: gui-watch-visual-feedback
description: >-
  Work with GUI applications in a fast visual feedback loop. Use whenever developing,
  refining, or styling GUI apps (e.g. Rust/Win32/Tauri/Flutter/Electron/Web overlays)
  where an external shell runs the app in watch/hot-reload mode (such as `cargo watch -x run`)
  and the agent autonomously captures and inspects rendered UI via screenshots to iteratively
  adjust margins, typography, layout, and visual appearance without manual user intervention.
---

# GUI Watch & Autonomous Visual Feedback Workflow

This skill defines the autonomous feedback loop procedure for developing and refining Desktop/GUI applications using hot-reload / watch mode and autonomous visual inspection.

## 1. Two Modes of Visual Capture

1. **Headless Offscreen Render Test (Preferred for Agent Autonomy)**:
   - Run a unit test that invokes the offscreen renderer (`render_hud(&mut pixmap)`) and saves the output to `target/hud_rendered.png` (e.g., `cargo test test_render_hud_with_reset_timestamps`).
   - Requires **no external terminal** and **no live window**, completing deterministically in a single command.
   - When running a release-profile test binary on Windows (`windows_subsystem = "windows"`) outside `cargo` to omit the `DEV` badge, pipe the executable call to `| Out-Default` in PowerShell so PowerShell waits synchronously for the GUI-subsystem process to finish writing the PNG before copying or viewing it.

2. **Live Watch Mode in External Shell (Optional for Live Window Tracking)**:
   - When testing live window tracking or real-time desktop behavior, an external shell can run `cargo watch -x run`.
   - Whenever source code is edited and saved, the live process is automatically recompiled and restarted.

## 2. The Autonomous Iteration Cycle

When tasked with adjusting layout, typography, margins, padding, colors, or visual hierarchy:

```
[ Code Edit & Save ]
        │
        ▼
[ Run Offscreen Render Test (or Watch Auto-Restart) ]
        │  e.g. `cargo test test_render_hud_with_reset_timestamps`
        ▼
[ Frame Snapshot Generated (`target/hud_rendered.png`) ]
        │
        ▼
[ view_file on Snapshot PNG ]
        │  (Multimodal visual inspection of layout & text clearance)
        ▼
[ Evaluate & Adjust Parameters ]
        │
        └─── Repeat until visually balanced
```

## 3. Best Practices for Visual Quality

- **Text & UI Clearances**:
  - Never allow text baselines or descenders to collide with progress tracks, borders, or hubs.
  - When in doubt, ensure at least 6–10px of visible breathing room between text labels and circular/arc boundaries.
- **Section Headers & Hierarchy**:
  - Center section headers relative to the panel width (`w / 2.0`) to provide bilateral symmetry for multi-column or dual meters.
  - Distribute meter centers symmetrically (`w / 2 - offset`, `w / 2 + offset`).
- **Aspect Ratio & Compactness**:
  - Keep HUD overlays as compact as possible while preserving comfortable padding to minimize desktop screen footprint.
