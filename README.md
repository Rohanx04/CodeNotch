# CodeNotch

A lightweight always-on-top HUD for Windows 10/11 that shows how much of each AI
coding assistant's usage limit you have burned, when the window resets, and
whether a background agent is generating, finished, or **waiting for your
approval**.

It is an open-source Windows alternative to the macOS
[codenotch](https://github.com/vinzdg/codenotch), rebuilt on Tauri v2 with a
Rust backend and a React frontend.

```
┌──────────────────────────────────┐
│  ◕  86%  Claude   ● ● ● ● ●      │   resting: a 190×32 pill, click-through
└──────────────────────────────────┘
                 ▼ hover
┌──────────────────────────────────┐
│ ◕ 86% of nearest limit    ⟳ 📌 ⚙ │
├──────────────────────────────────┤
│ ◕ Claude Code   NEEDS YOU        │
│   5h session ▓▓▓▓▓▓▓░░  86% 1h36m│
│   7d all     ▓▓▓░░░░░░  34% 3d 9h│
│   ● codenotch            39s ago │
├──────────────────────────────────┤
│ ◔ Cursor        WORKING          │
│ ...                              │
└──────────────────────────────────┘
```

## What it watches

| Provider | Source | What you get |
| --- | --- | --- |
| **Claude Code** | `GET /api/oauth/usage` with the token from `%USERPROFILE%\.claude\.credentials.json` or Windows Credential Manager; falls back to session transcripts under `%USERPROFILE%\.claude\projects\` | Real 5-hour and 7-day utilisation, reset times, plan, per-project sessions, and whether a session is parked on a permission prompt |
| **Cursor** | `%APPDATA%\Cursor\User\globalStorage\state.vscdb` and per-workspace databases, read without locking them | Plan, account, any cached request counters, live composer sessions per project |
| **Codex** | Rollout transcripts under `%USERPROFILE%\.codex\sessions\` (plus `~/.codex-<profile>`) | The rate-limit snapshot Codex records from the API, token totals, pending tool approvals |
| **GitHub Copilot** | Cached quota payloads under `%LOCALAPPDATA%\github-copilot\`, `~/.config/github-copilot\`, `~/.copilot\` | Plan, signed-in user, chat/completions/premium quota and reset date |
| **Ollama** | `http://127.0.0.1:11434/api/ps` | Resident models, VRAM vs system-RAM split, quantisation, context length |

Two rules hold across every adapter:

- **Read-only.** Nothing is written to, locked, or modified in another tool's
  state. The Cursor adapter parses the SQLite file format directly (including
  committed WAL frames) rather than opening a connection, so a running Cursor
  can never be disturbed and can never block us.
- **No invented numbers.** A failed collection degrades to a visible status —
  `stale`, `sign in`, `rate limited`, `error` — instead of a plausible-looking
  zero. Anything derived locally rather than reported by the provider is marked
  with a `~`.

## Window behaviour

The point of a HUD is that it never gets in the way, which on Windows means
getting four things right:

| Goal | How |
| --- | --- |
| Never steals focus from your IDE or terminal | `WS_EX_NOACTIVATE`, and `SWP_NOACTIVATE` on **every** move and resize |
| Stays out of Alt+Tab and the taskbar | `WS_EX_TOOLWINDOW`, with `WS_EX_APPWINDOW` cleared |
| Stays on top | `HWND_TOPMOST`, re-asserted on every poll (other topmost windows can displace it) |
| Doesn't swallow clicks while resting | `WS_EX_TRANSPARENT` toggled on collapse, off on expand |

Appearance uses `DwmSetWindowAttribute` for immersive dark mode, rounded corners
and an accent-tinted border, with the acrylic backdrop left to Tauri's own
window effect so the two mechanisms don't fight. All the Windows 11-era
attributes are best-effort: on Windows 10 they fail harmlessly and the CSS
fallback (`backdrop-filter`) carries the look.

Clicking a provider card raises that tool's window via `SetForegroundWindow`,
using the `AttachThreadInput` dance that Windows requires — without it the call
silently no-ops and the taskbar button just flashes.

## Requirements

- Windows 10 (1809+) or Windows 11, 64-bit
- [Rust](https://rustup.rs/) 1.82+ with the MSVC toolchain
- [Node.js](https://nodejs.org/) 20+
- **Microsoft Visual Studio C++ Build Tools** (the "Desktop development with
  C++" workload) — Rust's MSVC toolchain needs the linker
- **WebView2** — preinstalled on Windows 11 and current Windows 10; the
  installer bundles a bootstrapper otherwise

## Run it

```powershell
git clone https://github.com/Rohanx04/CodeNotch.git
cd CodeNotch

npm install
npm run tauri:dev
```

The notch appears pinned to the top-centre of your primary monitor. Hover it to
expand, click the pin to keep it open, and use the gear for settings. A tray
icon appears alongside it: left-click peeks the HUD, right-click gives you
show/hide, refresh, the config folder, and quit.

To produce an installer:

```powershell
npm run tauri:build
```

The NSIS and MSI packages land in
`src-tauri\target\release\bundle\`.

### Iterating on the UI without Windows

The frontend runs standalone in a browser against built-in sample data, which is
the fastest way to work on layout:

```bash
npm run dev      # http://localhost:1420
```

## Tests

The collection logic lives in `codenotch-core`, a crate with no Tauri or GUI
dependency, so its tests run on any host:

```bash
cd src-tauri/core
cargo test          # 149 tests: adapters, SQLite reader, layout, config, collector
cargo clippy --all-targets
```

The SQLite reader is checked against databases produced by real SQLite —
including a WAL that was deliberately left uncheckpointed, and a torn one — so
it is validated against the actual on-disk format rather than our assumptions
about it. Regenerate the fixtures with:

```bash
python3 scripts/gen_test_fixtures.py
```

Type-check the whole Windows application, including the Win32 layer, from any
platform:

```bash
rustup target add x86_64-pc-windows-msvc
cd src-tauri
cargo check --target x86_64-pc-windows-msvc
cargo clippy --target x86_64-pc-windows-msvc --all-targets
```

This works because the crate pulls in no C-compiled dependencies: TLS goes
through schannel via `native-tls`, and SQLite is parsed in pure Rust. The one
external tool it does need is `llvm-rc`, which `tauri-build` shells out to when
compiling the Windows resource file off-Windows (`sudo apt install llvm` on
Debian/Ubuntu); without it the build script panics with
`NotAttempted("llvm-rc")`.

Frontend:

```bash
npm run typecheck
npm run build
```

## Configuration

Settings live at `%APPDATA%\CodeNotch\config.json` and are editable from the
gear icon or by hand (unknown keys are ignored, missing ones get defaults, and a
corrupt file falls back to defaults rather than refusing to start).

| Key | Default | Notes |
| --- | --- | --- |
| `edge` / `edgeOffset` / `margin` | `top` / `0.5` / `0` | Which screen edge to pin to and where along it |
| `size` | `medium` | `small`, `medium`, `large` |
| `accent` | `#22d3ee` | Ring and border accent, `#rrggbb` |
| `monitor` | `{"kind":"primary"}` | Or `{"kind":"index","index":1}` |
| `alwaysExpanded` | `false` | Skip the collapsed pill entirely |
| `clickThroughWhenCollapsed` | `true` | Let clicks fall through while resting |
| `peekOnAttention` / `peekSecs` | `true` / `5` | Briefly expand when an agent finishes or needs you |
| `notifyOnThresholds` | `true` | Alert once at 80% and once at 100% per window |
| `resetAsCountdown` | `true` | `1h 36m` rather than a clock time |
| `launchAtLogin` | `false` | Adds an `HKCU\...\CurrentVersion\Run` entry |
| `poll.*` | 45–120s | Per-provider intervals, clamped to 5–3600s |
| `providers` | all `true` | Turn individual providers off |
| `ollamaUrl` | `http://127.0.0.1:11434` | Point at a remote or WSL daemon |

## Layout

```
src/                      React frontend (pill, expanded card, settings)
src-tauri/
  src/
    platform/             Win32: styles, docking, DPI, focus, registry
    hud.rs                Collapsed/expanded state and window placement
    commands.rs           The IPC surface
    poll.rs               Background polling loop and attention peeks
    tray.rs               Tray icon and menu
  core/                   codenotch-core -- no Tauri, fully unit-tested
    src/
      adapters/           One module per provider
      sqlite.rs           Read-only SQLite + WAL reader
      layout.rs           Edge anchoring and DPI maths
      collector.rs        Per-provider schedules, staleness, alerts
      model.rs            Domain types
scripts/                  Icon and test-fixture generators
```

## Privacy

Everything is local. The only network calls CodeNotch makes are to Anthropic's
usage endpoint with your existing Claude Code token, and to your own Ollama
daemon. No telemetry, no analytics, no third-party services. Credentials are
read but never copied, logged, or transmitted anywhere other than the provider
they belong to — the Copilot adapter, for instance, reads `apps.json` only to
learn *that* you are signed in and deliberately ignores the tokens beside it.

## Licence

MIT
