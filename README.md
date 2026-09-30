# CodeNotch

A lightweight always-on-top HUD for Windows 10/11 that shows how much of each AI
coding assistant's usage limit you have burned, when the window resets, and
whether a background agent is generating, finished, or **waiting for your
approval**.

It is an open-source Windows alternative to the macOS
[codenotch](https://github.com/vinzdg/codenotch), rebuilt on Tauri v2 with a
Rust backend and a React frontend.

```
                          ╭──────╮   resting: a black strip carved into the
                          │  ◕   │   screen edge, one ring per provider,
                          │ 86%  │   click-through so it never eats a click
   ┌────────────────────╮ │      │
   │ ✳ Claude Code      │ │  ◔   │
   │ Max                │◄┤ 41%  │   hover a ring: a card opens beside it,
   │ 5h session   1h 36m│ │      │   its tail pointing back at the ring
   │ ▓▓▓▓▓▓▓▓▓▓▓▓▓░░░░  │ │  ◕   │
   │ 86% Used           │ │ 72%  │
   └────────────────────╯ ╰──────╯
```

The ring is both gauge and activity light: it fills as the limit burns down
(green, then amber, then red), a cyan comet rides it while an agent is
generating, and it pulses amber when one is blocked on a `[y/N]` prompt. A
badge on its corner says when an agent needs you, has finished, or is broken.

With the optional [Claude Code hooks](#claude-code-hooks-optional) installed it
goes further: the Claude Code card shows each step a session takes as it takes
it, and a permission request appears on the notch with **Allow** and **Deny**,
so you can answer it without leaving what you are doing.

## What it watches

| Provider | Source | What you get |
| --- | --- | --- |
| **Claude Code** | `GET /api/oauth/usage` with the token from `%USERPROFILE%\.claude\.credentials.json` or Windows Credential Manager, whichever holds the fresher token; falls back to session transcripts under `%USERPROFILE%\.claude\projects\` | Real 5-hour and 7-day utilisation, reset times, plan, per-project sessions, and whether a session is parked on a permission prompt |
| **Cursor** | `%APPDATA%\Cursor\User\globalStorage\state.vscdb` and per-workspace databases, read without locking them | Plan, account, any cached request counters, live composer sessions per project |
| **Codex** | Rollout transcripts under `%USERPROFILE%\.codex\sessions\` (plus `~/.codex-<profile>`) | The rate-limit snapshot Codex records from the API, token totals, pending tool approvals |
| **GitHub Copilot** | Cached quota payloads under `%LOCALAPPDATA%\github-copilot\`, `~/.config/github-copilot\`, `~/.copilot\` | Plan, signed-in user, chat/completions/premium quota and reset date |
| **Gemini** | `%USERPROFILE%\.gemini\` — settings, OAuth creds, the signed-in account, and per-project logs under `tmp/` | Account, sign-in state, which projects are active and when |
| **Perplexity** | `%APPDATA%\Perplexity\` and `%APPDATA%\Comet\` | Plan and account where the app caches them |
| **Ollama** | `http://127.0.0.1:11434/api/ps` | Resident models, VRAM vs system-RAM split, quantisation, context length |

Gemini and Perplexity keep usage server-side and cache no quota locally, so
their cards say that rather than showing a ring. Both adapters will pick a
quota up automatically if a future build starts caching one.

Two rules hold across every adapter:

- **Read-only.** Nothing is written to, locked, or modified in another tool's
  state. (The one exception is opt-in and never silent: installing the Claude
  Code hooks adds entries to `~/.claude/settings.json`, after showing you the
  exact diff — see below.) The Cursor adapter parses the SQLite file format directly (including
  committed WAL frames) rather than opening a connection, so a running Cursor
  can never be disturbed and can never block us.
- **No invented numbers.** A failed collection degrades to a visible status —
  `stale`, `sign in`, `rate limited`, `error` — instead of a plausible-looking
  zero. Anything derived locally rather than reported by the provider is marked
  with a `~`.
- **A cached expiry is a hint, not a verdict.** These tools hold short-lived
  access tokens and renew them from a refresh token whenever they run, so a
  signed-in account spends most of its time with a lapsed timestamp on disk.
  Adapters therefore never conclude "signed out" from a stale `expiresAt`:
  Claude asks the endpoint and lets a 401 say so, and Gemini treats a refresh
  token as proof the CLI can renew itself.

## Window behaviour

The point of a HUD is that it never gets in the way, which on Windows means
getting five things right:

| Goal | How |
| --- | --- |
| Never steals focus from your IDE or terminal | `WS_EX_NOACTIVATE`, and `SWP_NOACTIVATE` on **every** move and resize |
| Stays out of Alt+Tab and the taskbar | `WS_EX_TOOLWINDOW`, with `WS_EX_APPWINDOW` cleared |
| Stays on top | `HWND_TOPMOST`, re-asserted on every poll (other topmost windows can displace it) |
| Never swallows a click meant for something else | click-through everywhere except over the strip and an open card |
| Opens on hover *despite* being click-through | the cursor is polled with `GetCursorPos` at 60 Hz on its own thread |
| Costs nothing while hidden | that thread parks on a condvar whenever the notch is hidden or tucked away |
| Survives monitors changing | displays are re-checked twice a second and the notch re-docks |
| Shows nothing but the notch | no window effect, no system backdrop, no frame border, no window rounding |

The click rows are the subtle ones. While the notch is on screen its window is a
fixed panel — the strip plus room for the tallest card beside it — so most of it
is empty desktop. The webview reports the shapes it has actually painted, and
the cursor poll clears `WS_EX_TRANSPARENT` only while the pointer is over one of
them (with a 14 px margin, so the flag is already off by the time a moving
pointer reaches a button). A `WS_EX_TRANSPARENT` window receives no mouse
messages at all — not even `mouseenter` — so the same poll is what tells the
notch the pointer arrived.

The last row is the one that makes it look built in. The window is bigger than
the notch — it holds the card alongside the strip, and the silhouette tapers
away at both ends — so anything the compositor draws against the *window
rectangle* shows up as a panel around the notch rather than as part of it. An
acrylic window effect frosts that whole rectangle and rims it in light;
`DWMWCP_ROUND` rounds and outlines it; a system backdrop fills it. All three are
therefore off, and the window is left genuinely transparent: the webview paints
the silhouette and the card, and every other pixel is desktop.

The rounding that *should* be there still is — the strip's by `NotchShape`, the
card's by its own `border-radius`. The accent lives on the rings, where the
webview paints it, rather than tinting a frame border.

`DwmSetWindowAttribute` is still used for immersive dark mode. All the Windows
11-era attributes are best-effort: on Windows 10 they fail harmlessly, and
there is nothing to fall back to because the look never depended on them.

Clicking a provider card raises that tool's window via `SetForegroundWindow`,
using the `AttachThreadInput` dance that Windows requires — without it the call
silently no-ops and the taskbar button just flashes.

**Hide when idle** (off by default) tucks the strip into the screen edge after
a quiet spell and shrinks the window to a 4 px wake strip along the edge; moving
the pointer onto it, or an agent starting, finishing or asking for permission,
brings the notch straight back. **Esc** closes an open card while the notch has
the keyboard; the notch never takes focus to get it.

## Motion

The notch should read as one object that moves, not a set of panels that
appear. Because its window keeps one size the whole time it is on screen,
nothing about the window has to change for anything to move, and it all runs
from one animation loop that stops itself the moment nothing is moving:

- **Opening and closing are different gestures.** A card springs out of the
  strip (a damped spring: a little life on arrival) and closes on a fixed
  340 ms curve with no overshoot, so it leaves cleanly. The close plays out in
  full; pointing back at the card mid-close springs it open again.
- **The card changes size by springing** when its content does, and **glides
  along the edge** between rings with the tail tracking it, rather than
  replaying its entrance.
- **Content crossfades.** Switching between a usage card, settings and a
  permission request fades the old view out and the new one in a beat later
  with a slight overshoot; provider to provider it is quicker. On first open
  the contents rise in a short stagger.
- **Every change of state gets a gesture on its ring**: a hop out of the edge
  when an agent starts waiting on you, a spin and a few sparks when one
  finishes, a shake when something breaks. A badge pops onto the ring's corner
  and a soft glow behind it takes the state's colour.
- **Work in progress shimmers**, and the Claude Code card's live steps roll
  through a three-line ticker that queues bursts instead of dropping them.
- **A peek counts down**: a thin bar shrinks through its last seconds.
- **On launch** the strip slides out of the edge and the arcs sweep up one
  after another; auto-hide slides it back in.
- **Short sound cues** (off in Settings): a card opening and closing, an agent
  finishing, a permission request, a limit crossing 80 % or 100 %. They are
  synthesised in code, not played from files.

`prefers-reduced-motion` is honoured with a blanket rule rather than a list of
selectors, so motion added later is covered by default; the animation loop
jumps straight to its targets instead of animating.

## Claude Code hooks (optional)

Transcripts say what a Claude Code session *was* doing, a poll late and by
inference. Hooks say what it is doing now: which tool it is running on which
file, that it is blocked on a permission prompt, that the turn just ended.

Settings → **Claude Code hooks** → **Install hooks…** shows the exact diff to
`%USERPROFILE%\.claude\settings.json` and where the dated backup will go;
nothing is written until you press **Write settings.json** under that diff, and
the write is refused if the file changed in the meantime. Other settings and
other tools' hooks are left alone, and **Remove hooks…** takes out only
CodeNotch's entries.

The hooks run `codenotch-hook.exe`, a small relay copied to
`%LOCALAPPDATA%\CodeNotch\bin\` at launch, which forwards each event over a
per-user named pipe. **Claude Code is never blocked by it**: if CodeNotch is not
running the relay exits at once, every step runs under a deadline, and only a
permission request waits for an answer. That request shows on the notch with
exactly what Allow authorises (`Bash · cargo publish`, not just `Bash`); one
request at a time, and if nobody answers within 108 seconds — or the notch is
paused or hidden — the terminal asks as usual. Nothing is ever approved without
a click.

**Pause** in the tray menu stops collection, peeks and sounds, and hands any
permission request straight to the terminal. It lasts until you unpause or quit.

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

The notch appears against the right edge of your primary monitor, one ring per
provider. Hover a ring for its detail card; click one to bring that tool's
window to the front. A tray icon appears alongside it: left-click peeks the HUD,
right-click gives you show/hide, keep expanded, pause, refresh, the config
folder, and quit.

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

A few query switches stand in for the backend: `?approval` brings up a
permission request, `?peek` an attention peek with its countdown, `?hide`
auto-hide tucking the strip away, and `?gestures` cycles a ring through
working, finished and waiting (and another through broken) so every reaction
can be watched.

## Tests

The collection logic lives in `codenotch-core`, a crate with no Tauri or GUI
dependency, so its tests run on any host:

```bash
cd src-tauri/core
cargo test          # 218 tests: adapters, SQLite reader, layout, config, collector, hooks
cargo clippy --all-targets
```

The hook relay's parsing and output are tested the same way:

```bash
cd src-tauri
cargo test -p codenotch-hook
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
cargo check --target x86_64-pc-windows-msvc --workspace
cargo clippy --target x86_64-pc-windows-msvc --workspace --all-targets
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
gear at the end of the strip or by hand (unknown keys are ignored, missing ones get defaults, and a
corrupt file falls back to defaults rather than refusing to start).

| Key | Default | Notes |
| --- | --- | --- |
| `edge` / `edgeOffset` / `margin` | `right` / `0.5` / `0` | Which screen edge to pin to and where along it |
| `size` | `medium` | `small`, `medium`, `large` |
| `accent` | `#22d3ee` | Ring and border accent, `#rrggbb` |
| `monitor` | `{"kind":"primary"}` | Or `{"kind":"index","index":1}` |
| `alwaysExpanded` | `false` | Keep a detail card open rather than waiting for hover |
| `peekOnAttention` / `peekSecs` | `true` / `5` | Briefly expand when an agent finishes or needs you |
| `autoHide` / `autoHideSecs` | `false` / `60` | Tuck the notch into the edge after this long with nothing going on |
| `sound` / `soundVolume` | `true` / `0.12` | Sound cues, and their volume (0–0.2) |
| `notifyOnThresholds` | `true` | Alert once at 80% and once at 100% per window |
| `resetAsCountdown` | `true` | `1h 36m` rather than a clock time |
| `launchAtLogin` | `false` | Adds an `HKCU\...\CurrentVersion\Run` entry |
| `clickThroughWhenCollapsed` | `true` | Retired: click-through now follows the painted shapes. Still read, so old files load |
| `poll.*` | 45–120s | Per-provider intervals, clamped to 5–3600s |
| `providers.gemini` etc. | `true` | One switch per provider |
| `providers` | all `true` | Turn individual providers off |
| `ollamaUrl` | `http://127.0.0.1:11434` | Point at a remote or WSL daemon |

## Layout

```
src/                      React frontend (strip, cards, settings)
  lib/motion.ts           Springs and the close curve
  lib/sound.ts            Synthesised sound cues
src-tauri/
  src/
    platform/             Win32: styles, docking, DPI, focus, registry
    hud.rs                Window placement, hover and click-through, auto-hide
    commands.rs           The IPC surface
    poll.rs               Polling loop, live overlay, attention peeks and cues
    hooks.rs              Claude Code hook install and events
    approvals.rs          Permission requests answered from the notch
    pipe.rs               The named pipe the hook relay talks to
    tray.rs               Tray icon and menu
  hook/                   codenotch-hook -- the relay Claude Code runs
  core/                   codenotch-core -- no Tauri, fully unit-tested
    src/
      adapters/           One module per provider
      sqlite.rs           Read-only SQLite + WAL reader
      layout.rs           Panel geometry, edge anchoring, DPI, hit testing
      presence.rs         Auto-hide state machine
      live.rs             Live Claude Code sessions from hook events
      hook_settings.rs    Safe settings.json install (backup, merge, diff)
      collector.rs        Per-provider schedules, staleness, alerts
      model.rs            Domain types
scripts/                  Icon and test-fixture generators
```

## Privacy

Everything is local. The only network calls CodeNotch makes are to Anthropic's
usage endpoint with your existing Claude Code token, and to your own Ollama
daemon. Claude Code hook events travel over a named pipe on your own machine (remote
connections are refused, and the relay checks the pipe belongs to your own
account before sending anything) and are never stored. No telemetry, no analytics, no third-party services. Credentials are
read but never copied, logged, or transmitted anywhere other than the provider
they belong to — the Copilot adapter, for instance, reads `apps.json` only to
learn *that* you are signed in and deliberately ignores the tokens beside it.

## Licence

MIT.

Some of the motion and hook plumbing is adapted from
[Coucou](https://github.com/Louis-CFM/coucou) by Louis Raillé, whose source
code is MIT-licensed (`LICENSES/Coucou-MIT.txt`): the spring and close-curve
helpers, the rolling step ticker, the hook relay and its named-pipe protocol,
and the `settings.json` merge. Coucou's name, its Mochi character, its icon and
its sounds are not part of that licence, and none of them are used here.

Six of the seven provider marks on the strip come from [Simple
Icons](https://simpleicons.org), whose icon data is released under CC0 1.0
(`LICENSES/CC0-1.0.txt`); Codex's is drawn by hand, since OpenAI had theirs
withdrawn from that set. Each logo remains the
trademark of its owner and is used here only to identify which tool a ring
belongs to.
