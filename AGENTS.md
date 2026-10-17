# AGENTS.md

Instructions for coding agents and contributors working on this repository.
User-facing docs live in [README.md](README.md). Keep README focused on end users and put development details here.

## Project overview

A native Win32 HUD overlay (Rust) that shows Google Antigravity quota on top of the Antigravity window.

| Module | Role |
|---|---|
| `src/main.rs` | Entry point, CLI flags (`--register-startup` etc.), main loop (message pump + window tracking + quota refresh) |
| `src/quota.rs` | Finds `language_server.exe`, reads the CSRF token from its PEB command line, finds its listening port, calls `RetrieveUserQuotaSummary` |
| `src/tracker.rs` | Finds the `Antigravity.exe` main window (HWND) and its bounds |
| `src/overlay.rs` | Layered HUD window, `tiny-skia` + `cosmic-text` rendering, dimensionless `DesignTokens`, fade animation |
| `src/tray.rs` | System tray icon (dynamic Gemini 5h ring), tooltip, context menu, restart/exit |
| `src/startup.rs` | Auto-start via `HKCU\...\CurrentVersion\Run` |
| `src/runtime.rs` | Distinguishes `cargo run` / debug binary / release binary |

### Key behaviors

- **Main loop** runs at ~60 fps (`sleep(16ms)`) with `PeekMessageW`. `WM_QUIT` ends the process (tray → Exit posts it).
- **Quota refresh triggers**: every 5 minutes, plus `request_refresh()` on tray left/right click and on HUD hidden → shown transitions. Requests are debounced to at least 3 seconds apart.
- **Window tracking**: the HUD is placed at the bottom-right of the Antigravity window. Moves/resizes fade it out, and it comes back after 200 ms of no movement. It also fades out when the cursor is within 20 px of the HUD.
- **Rendering**: full vector redraw only on data updates. Fades only call `UpdateLayeredWindow` with a new constant alpha on the cached DIBSection.
- **Runtime mode**: `cargo run` / debug builds show a `DEV` badge on the HUD and `[DEV]` in the tray tooltip. Release builds use `windows_subsystem = "windows"`, so there is no console window. CLI flags call `AttachConsole(ATTACH_PARENT_PROCESS)` so their output still shows up.
- **Startup registration** always points to the release binary. If you register from `target\debug`, it uses the sibling `target\release` exe when one exists.

## Environment setup (Windows 11)

```powershell
# Rust toolchain + C linker (WinLibs LLVM/GCC)
winget install --id Rustlang.Rustup -e --accept-source-agreements --accept-package-agreements
winget install --id BrechtSanders.WinLibs.POSIX.UCRT.LLVM -e --accept-source-agreements --accept-package-agreements
$env:Path = "$env:USERPROFILE\.cargo\bin;" + $env:Path

# Windows GNU target as default
rustup toolchain install stable-x86_64-pc-windows-gnu
rustup default stable-x86_64-pc-windows-gnu

# Quality tools
rustup component add rustfmt clippy
cargo install cargo-binstall --locked
cargo binstall -y cargo-machete cargo-audit cargo-deny cargo-watch
```

| Tool | Purpose |
|---|---|
| rustfmt | Formatting |
| clippy | Lints (keep at zero warnings) |
| cargo-machete | Finds unused dependencies |
| cargo-audit | RustSec vulnerability audit |
| cargo-deny | Checks licenses, advisories, and duplicate crates (`deny.toml`) |
| cargo-watch | Hot-reload dev loop |

## Quality gate (run before every commit)

```powershell
cargo fmt --check; cargo clippy --all-targets --all-features; cargo machete; cargo audit; cargo deny check; cargo test
```

All of these must pass with exit code 0 and no warnings. Also run `cargo build --release` when you touch platform or windowing code.

## Dev loop

```powershell
cargo watch -x run   # rebuild and restart on save
```

For GUI and visual work, follow the workspace skills:

- `.agents/skills/gui-watch-visual-feedback/` — visual feedback loop (render, inspect, adjust)
- `.agents/skills/gui-dimensionless-design/` — spacing and typography rules

### Rendering the HUD to a PNG

`overlay::tests::test_render_hud_with_reset_timestamps` renders `render_hud(&mut pixmap)` offscreen with sample data directly to `target/hud_rendered.png`. Agents can run `cargo test test_render_hud_with_reset_timestamps` and inspect `target/hud_rendered.png` via `view_file` without needing a live window or an external `cargo watch` terminal.

To update the README screenshot (`docs/hud.png`), render it **without** the DEV badge. That means running a release-profile test binary directly outside of cargo (since cargo sets `CARGO` / `CARGO_MANIFEST_DIR`, which triggers dev mode):

```powershell
$t = cargo test --release --no-run --message-format=json 2>$null | ConvertFrom-Json |
  Where-Object { $_.executable -and $_.profile.test } | Select-Object -First 1
& $t.executable test_render_hud_with_reset_timestamps | Out-Default
Copy-Item target\hud_rendered.png docs\hud.png -Force
```

> **Note:** Release binaries use `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`. In PowerShell, invoking a `windows` subsystem `.exe` with `&` returns immediately (asynchronously) unless its output is piped. Always pipe to `| Out-Default` (or `| Out-Null`) so PowerShell waits for `target/hud_rendered.png` to finish writing before `Copy-Item` runs.

## Design rules

- **No hardcoded layout numbers.** All spacing and font sizes go through `DesignTokens` (`base_space`, `base_text` × multipliers), per the `gui-dimensionless-design` skill. Keep the token hierarchy test (`test_dimensionless_tokens_hierarchy_and_scales`) passing.
- Gauge colors follow `get_quota_color` (green → amber → red as quota drops).
- The HUD must stay click-through (`WS_EX_TRANSPARENT`) and must never take focus.

## Gotchas

- **Release exe locked**: if the user is running the release build (e.g. via auto-start), `cargo build --release` fails with `os error 5`. Stop it first: `Stop-Process -Name google-ai-usage-monitor`.
- **No stdout in release**: the release build has no console. Use `cargo run` (or a debug build) to see logs.
- **Launching the release exe from an agent shell**: processes started with `Start-Process` from the agent's terminal can be killed together with the parent job. To leave a detached instance running:
  ```powershell
  $exe = (Resolve-Path target\release\google-ai-usage-monitor.exe).Path
  Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{ CommandLine = $exe }
  ```
- **Never commit machine-specific paths** (e.g. user profile or agent scratch directories). Put test outputs under `target/`.
- The quota API is undocumented. Handle missing fields and failures gracefully: keep the last values and don't panic.

## Conventions

- Code comments are written in Japanese. Keep existing comments unless they are wrong.
- Commit messages use Conventional Commits (`feat:`, `fix:`, `refactor:`, `docs:` …).
