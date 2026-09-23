<div align="center">

# MultiAgents Manager

**Unified Management Platform for Multi-Agent Programming Tools**

A desktop app to monitor, notify, jump to, and manage Claude Code / Codex CLI / OpenCode / OpenClaw / Kimi Code / WorkBuddy / ZCode / dsh sessions

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Tauri v2](https://img.shields.io/badge/Tauri-v2-blue?logo=tauri)](https://v2.tauri.app/)
[![React 19](https://img.shields.io/badge/React-19-61DAFB?logo=react)](https://react.dev/)

English · [中文](README.md)

</div>

---

## Features

### Session Monitoring Dashboard

Real-time traffic-light status board for all active AI coding tool sessions.

| Status    | Meaning                |
| --------- | ---------------------- |
| 🔴 Red    | Waiting for user input |
| 🟡 Yellow | Processing / Thinking  |
| 🟢 Green  | Idle / Finished        |

- Auto-discovers running **Claude Code**, **Codex CLI/APP**, **OpenCode**, **OpenClaw**, **Kimi Code**, **WorkBuddy**, **ZCode**, and **dsh** sessions
- Distinguishes CLI vs. desktop APP form: APP sessions support session-level deep-link jumps (`workbuddy://chat/<id>`, `codex://threads/<id>`, with APP-foreground fallback) and persistent unread cards (kept across restarts, cleared when the host exits); dsh (web-hosted) jumps focus/open the dsh tab in your browser (macOS)
- Shows project name, git branch, last message preview, CPU usage, runtime
- Sorts by priority: waiting → running → idle
- System tray icon reflects aggregate status (🔴/🟡/🟢)

### Remote Access & Mobile Board (LAN / Quick Tunnel / Named Tunnel)

Open the same eight-tool session board from your phone browser. Settings → Remote Access offers **three connection methods, each with its own switch and usable simultaneously**:

| Method                          | When to use                                                   | Notes                                                                                                                                                                                                 |
| ------------------------------- | ------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 📶 LAN                          | Phone and computer on the same WiFi / cable (**recommended**) | Fastest; requires the access PIN                                                                                                                                                                      |
| 🔀 Quick tunnel                 | You're away and your phone is on cellular                     | Public, no signup (Cloudflare); **address changes every time it's enabled**                                                                                                                           |
| 🌐 Named tunnel (custom domain) | You want a stable entry point                                 | Needs a Cloudflare account (free plan works) + a Tunnel Token; the address is **permanent** once configured — MAM downloads and supervises cloudflared for you, with a step-by-step guide in Settings |

A "Local" channel is always on alongside the master switch: loopback-only, no PIN, and it doubles as the relay endpoint for tunnel traffic (not used by phones).

![Remote Access · desktop settings page](docs/images/remote-settings-v0.5.0.png)

How to connect: turn on "Enable Remote Access" → enable the channel you want → click its card to reveal the address / QR code (**the link already contains the PIN, so scanning fills it in automatically**) → open it on your phone (**the URL must end with `/m`**). The PIN applies to LAN and both tunnels: enter it once per device for **180 days**, and after a PIN change every device must re-enter it. Paired devices can be renamed or kicked from the "Paired Devices" list (up to 10).

**Live board**: session status changes are pushed to your phone within ~2 seconds (SSE as the primary channel) — banner, chime, and vibration, with a tap taking you straight to that session; if the stream drops it degrades to 3-second polling automatically. Transition deduplication uses the same server-side logic as desktop notifications.

**Session detail page** (ZCode-style conversation view, unified across all eight tools):

- Expand details while running (thinking / tool calls are collapsible), auto-collapse to the final summary once a turn completes
- Markdown rendering (headings / lists / tables / code highlighting); project file paths in the text become clickable links
- **File panel**: aggregates the files a session touched, newest first, with document/image filters and a **200 / 500 / 1000 message** look-back range; **modified** files are highlighted and **read-only** ones are neutral (both previewable), and tapping the secondary line reveals the full path
- **File preview**: markdown / syntax highlighting / inline images; three switchable layouts — side-by-side (chat left, file right), stacked, and fullscreen overlay — with a draggable splitter; readable scope is the session project directory plus your home directory (sensitive directories such as keys are always refused)
- **Message bookmarks**: mark a spot to revisit with a colored dot (up to 10, one per color), jump back by tapping it, delete individually or clear all; stored in the browser — refresh-proof, cleared when MAM restarts or the tab is closed
- **Font size**: 50% / 75% / 100% / 125% for message and file text only
- **Archived history**: finished or unreachable sessions stop cluttering the board — registry-based archiving (history page lazy-loads 1/3/7 days), tap a card for a read-only detail view with one-tap reactivation; board cards can be closed/archived (CLI sessions stop their process, APP sessions get a true "close")

### Remote Control · send messages & approve from your phone (v0.5.0, experimental)

Not just looking — you can **act**: send messages, approve tool calls, answer questions, and switch permission modes from your phone (currently **Claude Code / Codex CLI / Kimi Code / OpenCode**, experimental):

- **Send & queue**: messages queue automatically while a session is busy, "send now" interrupts and jumps the queue (per-tool key sequences measured on real machines), and queued messages can be retracted. Every message uses **key-sequence injection with a screen-read receipt** (character-by-character tail verification) — a non-delivery is reported honestly, never faked as success
- **Remote approvals**: tool-call approval (allow/deny), plan confirmation (plan body plus terminal dialog options synced live from the screen, direct digit selection)
- **Remote Q&A**: single-choice, multi-select, free-form answers, per-question progression with a confirmation card; multi-select taps only toggle, "next question" advances explicitly — the phone and the terminal always stay on the same question
- **Permission modes**: kimi's three-tier menu with two-key navigation and a mode receipt; codex switches via the terminal menu single-select
- **Guards**: while a dialog is pending, ordinary message injection is blocked (to avoid answering the default by accident); input-line residue is detected to prevent double sends; dangerous keys (such as Ctrl+C, which quits OpenCode) are blacklisted

|                Session detail · messages                |              Send · queue & jump               |
| :-----------------------------------------------------: | :--------------------------------------------: |
| ![Session detail](docs/images/mobile-detail-v0.5.0.png) |  ![Send](docs/images/mobile-send-v0.5.0.png)   |
|                  **Approvals · plan**                   |             **Q&A · multi-select**             |
|   ![Approvals](docs/images/mobile-approve-v0.5.0.png)   | ![Q&A](docs/images/mobile-question-v0.5.0.png) |

### Foxbell Desktop Pet

A talking fox companion that lives in the corner of your screen and watches every session in real time.

![Foxbell Desktop Pet](docs/images/foxbell-pet.png)

- Status cards above the pet mirror the dashboard: 🔴 waiting / 🟡 running / 🟢 finished — click a card to jump to its terminal
- Voice alerts (31 built-in clips): playful nudges on waiting approvals, cheers on completion, small talk on double-click, subtitles synced to audio length
- Drag physics: pinned-to-cursor dragging, gravity fall on release, throw inertia, squash-and-bounce landing (optional)
- Single-click waves, double-click talks, right-click menu: sound / subtitles / physics / always-on-top / size / per-scene action binding / hide
- Dashboard integration: takes over completion chimes, suppresses toast popups while always-on-top; toggle from the dashboard 🦊 button, system tray, or settings

#### External Pets

Since v0.3.0 the pet format is open — Foxbell is no longer the only companion:

- **Import custom pets**: from a local zip / directory, or download from the Petdex online repository; manifest structure, frame rate, dimensions and voice manifests are fully validated
- **Manage panel**: import / edit description / rename / delete / one-click hot swap — no app restart needed; the active pet is auto-restored after deletion or switching
- **Capability gating**: pets without voices gracefully degrade to animation-only (transient actions kept); voice capabilities stay in two-way sync

### Desktop Notifications & Sound Alerts

- Color-change-based notifications (red↔yellow↔green) with deduplication
- Web Audio API chimes — no audio files needed
- Configurable on/off toggle in settings
- Clickable notifications with "View Session" action to jump to terminal

### Quick Terminal Jump

Click a session card to instantly focus the corresponding terminal tab:

| Terminal     | Support                            |
| ------------ | ---------------------------------- |
| iTerm2       | ✅ AppleScript                     |
| Terminal.app | ✅ AppleScript                     |
| tmux         | ✅ pane selection + terminal focus |
| Wayland      | ❌ Graceful fallback message       |

Terminal tools (Claude Code / Codex CLI / OpenCode / Kimi Code) resolve through process-tree and window-content disambiguation; **same-project dual-open jumps land directly**: for Kimi / OpenCode, the window title is matched against the session title (kimi `state.json` title / OpenCode DB title) after normalization — a unique hit locks onto the window, so dual terminals no longer raise a picker. On Windows, resolution also stamps a one-shot identity marker onto the target terminal title (` — MAM:xxxxxxxxxxxx`, cleared automatically after focus) for positive locking; markers never stack, and when the card↔terminal pairing is uncertain (same-project multi-open) the app **raises a picker rather than risking the wrong window**, and focus refusals surface an explicit error instead of failing silently.

Desktop APP tools (Codex APP, WorkBuddy) support deep-link jumps: `codex://threads/<id>`, `workbuddy://chat/<id>` (session-level). The handler is verified before dispatch and foregrounding is verified after; on failure it falls back to APP-level focus (macOS AppleScript / Windows nearest-ancestor) without marking the session read. ZCode is a single-window multi-tab app — its jump simply focuses the unique window (cards carry the host pid, zero ambiguity). dsh's jump focuses (or opens) the dsh web tab in your browser.

### Extension Resource Management

Unified repository for Skills, MCP servers, and Plugins across tools:

- **Skills**: Symlink (Unix) / Junction (Windows) mapping to each tool's skill directory
- **MCP Servers**: Auto-format conversion — JSON (Claude / Kimi / WorkBuddy) / TOML (Codex) / JSONC (OpenCode) / nested JSON subtree (ZCode: `mcp.servers`; read-modify-write touches only that subtree, preserving unknown keys and original key order)
- **Plugins**: File/config hybrid management
- Auto-import existing skills on first launch (from per-tool directories such as `~/.claude/skills/`, `~/.codex/skills/`, `~/.config/opencode/skills/`, plus the shared directory `~/.agents/skills/`)
- `~/.agents/skills/` is a **read-only shared import source** (source label `agents-shared`): MAM only scans it into the repository (no tool attribution, no linking); tools that follow the open standard, such as codex / zcode, read this directory directly
- Rescan button for discovering newly installed skills

### Preset Groups

Bundle Skills + MCP servers + Plugins into named presets and apply/deactivate in one click:

- One-click apply to any tool — auto-adapts to each tool's config format
- Partial success handling: reports failures without rolling back successful items
- Conflict detection: skips already-existing resources
- System tray menu integration for quick switching

### Sub-Agent Resource Allocation

For multi-agent tools (Hermes, OpenCode, etc.), allocate resource subsets to sub-agents:

- Sub-agent allocation is constrained to the tool-level enabled range
- Tool-level disable cascades down to all sub-agents

### Tool Toggle Management

A dedicated settings section to decide which tools MAM monitors and manages:

- Row-style toggle list: icon + name + installed badge; changes are staged locally and batch-saved, with a confirmation dialog listing restore/rollback items and an unsaved-changes leave guard
- Unchecking = full restore: symlinks become real files, MCP entries are removed from tool configs, unread cards are cleared; the SSOT repository and DB assignments are kept, and re-checking rebuilds everything per the original assignments (partial failures auto-rollback — re-saving retries idempotently)
- Unchecked tools are fully hidden: session scanning skips them, notifications are muted, resource/preset UIs hide them, and guarded commands return structured, localized errors

---

## Tool Support Matrix

Capabilities across the 8 terminal-class AI coding tools (✅ supported / ◐ partial / 🧪 experimental / ❌ not supported):

| Capability                         | Claude Code | Codex CLI | OpenCode | OpenClaw | Kimi Code    | WorkBuddy | ZCode           | dsh         |
| ---------------------------------- | ----------- | --------- | -------- | -------- | ------------ | --------- | --------------- | ----------- |
| Session monitoring                 | ✅          | ✅        | ✅       | ✅       | ✅           | ✅        | ✅              | ✅          |
| Desktop notifications              | ✅          | ✅        | ✅       | ✅       | ✅           | ✅        | ✅              | ✅          |
| Skill management                   | ✅          | ✅        | ✅       | ✅       | ✅           | ✅        | ✅              | ◐ read-only |
| MCP management                     | ✅ JSON     | ✅ TOML   | ✅ JSONC | ✅ JSON  | ✅ JSON      | ✅ JSON   | ✅ JSON subtree | ❌          |
| Plugin management                  | ✅          | ✅        | ✅       | ✅       | ◐ file-based | ❌        | ❌              | ❌          |
| Status hooks                       | ✅          | ✅        | ❌       | ❌       | ✅           | ❌        | ❌              | ❌          |
| Mobile · message viewing           | ✅          | ✅        | ✅       | ✅       | ✅           | ✅        | ✅              | ✅          |
| Mobile · file preview              | ✅          | ✅        | ✅       | ✅       | ✅           | ✅        | ✅              | ✅          |
| Mobile · send messages (injection) | 🧪          | 🧪        | 🧪       | ❌       | 🧪           | ❌        | ❌              | ❌          |

**Mobile remote control (v0.5.0, experimental)**: message viewing and file preview cover all 8 tools; **sending messages to CLI sessions from your phone is experimental**, currently supporting Claude Code / Codex CLI / OpenCode / Kimi Code. Messages are injected as keystrokes into the terminal and verified by screen-reading receipts (no false "delivered"); queueing, interrupt-and-jump, remote approvals, question answering, and permission-mode switching are included. See the Chinese README for illustrated walkthroughs and per-tool limitations.

---

## Tech Stack

| Layer              | Technology                                                                               |
| ------------------ | ---------------------------------------------------------------------------------------- |
| Desktop Framework  | [Tauri v2](https://v2.tauri.app/) (Rust)                                                 |
| Frontend           | [React 19](https://react.dev/) + [TypeScript](https://www.typescriptlang.org/)           |
| UI Components      | [shadcn/ui](https://ui.shadcn.com/) (Radix UI)                                           |
| Styling            | [Tailwind CSS v4](https://tailwindcss.com/)                                              |
| State Management   | [Zustand](https://zustand-demo.pmnd.rs/)                                                 |
| i18n               | [i18next](https://www.i18next.com/) (Chinese / English)                                  |
| Database           | [SQLite](https://www.sqlite.org/) (via [rusqlite](https://github.com/rusqlite/rusqlite)) |
| Process Monitoring | [sysinfo](https://github.com/GuillaumeGomez/sysinfo)                                     |

## Architecture

```
src-tauri/src/
├── adapter/           # Agent adapter trait + per-tool implementations
│   ├── claude.rs      #   Claude Code (JSONL + Hook)
│   ├── codex.rs       #   Codex CLI/APP (JSONL + Hook)
│   ├── opencode.rs    #   OpenCode (SQLite)
│   ├── openclaw.rs    #   OpenClaw (state.json)
│   ├── kimi.rs        #   Kimi Code (session_index + wire.jsonl)
│   ├── workbuddy.rs   #   WorkBuddy (heartbeat-driven + JSONL)
│   ├── zcode.rs       #   ZCode (host detection + SQLite session aggregation)
│   ├── dsh.rs         #   dsh (web host + zstd multi-frame logs)
│   └── mod.rs         #   AgentAdapter trait + tool registry + session discovery scheduler
├── monitor/
│   ├── process.rs     #   Process discovery (sysinfo scan)
│   ├── claude_parser.rs   # Claude parser (message.role protocol)
│   ├── codex_parser.rs    # Codex parser (rollout JSONL protocol)
│   ├── opencode_parser.rs # OpenCode SQLite parser
│   ├── openclaw_parser.rs # OpenClaw state.json parser
│   ├── kimi_parser.rs     # Kimi Code parser (session_index + wire.jsonl)
│   ├── workbuddy_parser.rs # WorkBuddy parser (heartbeat + JSONL tail)
│   ├── zcode_parser.rs    # ZCode parser (tasks-index + session/message/part dual SQLite)
│   ├── dsh/           #   dsh parser (projcache + zstd logs + status/preview, 5 modules)
│   ├── jsonl.rs       #   Shared JSONL reading (tail read, file enumeration)
│   ├── cwd.rs         #   cwd normalization (process ↔ session matching)
│   ├── git.rs         #   GitHub URL lookup (in-process cache)
│   ├── path_codec.rs  #   Claude projects dir-name codec
│   ├── project.rs     #   Project name extraction + cwd shape validation
│   ├── status.rs      #   Pure-message status determination
│   └── hooks.rs       #   Hook registration + event file reader
├── services/          #   Business services split by domain
│   ├── skill/         #   Skill install/enable/disable + auto-import
│   ├── resource/      #   Resource scan, SSOT import, link sync
│   ├── mcp/           #   MCP config writer (JSON/TOML/JSONC)
│   ├── preset/        #   Preset apply/deactivate + compatibility check
│   ├── plugin/        #   Plugin management
│   └── manifest/      #   Extension manifest validation + update check
├── linker/
│   ├── mod.rs         #   Symlink/Junction management + security checks
│   ├── detector.rs    #   Tool installation detection
│   ├── layer2.rs      #   Layer 2 tool-level active directory
│   └── layer3.rs      #   Layer 3 sub-agent-level active directory
├── commands/          #   Tauri IPC commands split by module
├── database/          #   SQLite data layer (schema/migration/dao)
├── session/           #   Session model + status enum
├── window/            #   Terminal focus (iTerm2 / Terminal.app / tmux)
├── plugins/
│   └── system_tray.rs #   System tray with status + preset menu
└── lib.rs             #   App entry + plugin registration

src/
├── pages/             #   Home / Settings / About
├── components/
│   ├── SessionCard.tsx #   Session card with status light
│   ├── SessionGrid.tsx #   Dashboard grid
│   ├── ExtensionList.tsx # Dual-view (byKind/byTool) resource management
│   ├── ResourceByKindView.tsx # Skills/MCP/Plugins three-section view
│   ├── ResourceByToolView.tsx # Four-tool card view
│   ├── ImportDialog.tsx  #   Native resource scan & import
│   ├── CompatibilityDialog.tsx # Preset compatibility check
│   ├── PresetList.tsx  #   Preset group CRUD
│   └── ui/            #   shadcn/ui primitives
├── hooks/             #   useSessions, useNotification, useUpdater
├── stores/            #   Zustand session store
├── lib/               #   Audio, shortcut, updater, window utils
├── i18n/              #   Chinese + English locales
└── types/             #   TypeScript type definitions
```

---

## Getting Started

### Prerequisites

- [Node.js](https://nodejs.org/) ≥ 18
- [pnpm](https://pnpm.io/) ≥ 8
- [Rust](https://www.rust-lang.org/tools/install) ≥ 1.77
- [Tauri v2 CLI](https://v2.tauri.app/start/prerequisites/)

### Install & Run

```bash
# Clone the repository
git clone https://github.com/jarvislee90s-dot/MultiAgents-Manager.git
cd MultiAgents-Manager

# Install frontend dependencies
pnpm install

# Start development mode
pnpm tauri:dev
```

### Build

```bash
# Build release binary (Windows NSIS installer)
pnpm tauri:build
```

### Lint & Format

```bash
pnpm check        # format:check + lint + build
pnpm format       # auto-format with Prettier
pnpm lint         # ESLint check
pnpm lint:fix     # ESLint auto-fix
```

---

## Configuration

The app stores its data in `~/.mam/`:

| Path                          | Purpose                                                        |
| ----------------------------- | -------------------------------------------------------------- |
| `~/.mam/mam.db`               | SQLite database (settings, extensions, presets, session cache) |
| `~/.mam/skills/`              | Global skill repository                                        |
| `~/.mam/mcp/`                 | Global MCP server configs                                      |
| `~/.mam/hooks/status-hook.sh` | Shared Hook script for status events                           |
| `~/.mam/events/`              | Hook event files (auto-cleaned, 30s TTL)                       |

### Supported Tool Configs

| Tool        | Skill Directory              | MCP Config                         | MCP Format                          | Hook Support                                             |
| ----------- | ---------------------------- | ---------------------------------- | ----------------------------------- | -------------------------------------------------------- |
| Claude Code | `~/.claude/skills/`          | `~/.claude.json`                   | JSON                                | ✅ (PascalCase)                                          |
| Codex CLI   | `~/.codex/skills/`           | `~/.codex/config.toml`             | TOML                                | ✅ (camelCase)                                           |
| OpenCode    | `~/.config/opencode/skills/` | `~/.config/opencode/opencode.json` | JSONC                               | ❌                                                       |
| OpenClaw    | `~/.openclaw/skills/`        | N/A                                | N/A                                 | ❌                                                       |
| Kimi Code   | `~/.kimi-code/skills/`       | `~/.kimi-code/mcp.json`            | JSON                                | ❌ (status parsed from wire)                             |
| WorkBuddy   | `~/.workbuddy/skills/`       | `~/.workbuddy/mcp.json`            | JSON                                | ❌ (status derived from heartbeat + JSONL)               |
| ZCode       | `~/.zcode/skills/`           | `~/.zcode/cli/config.json`         | JSON (nested `mcp.servers` subtree) | ❌ (status derived from SQLite message-stream tail)      |
| dsh         | `~/.dsh/skills/`             | N/A (probed unsupported)           | N/A                                 | ❌ (status derived from lock cross-check + event stream) |

> Note: `~/.agents/skills/` is the cross-tool shared directory of the Agent Skills open standard (read directly by codex / zcode and other compliant tools). MAM's skill activation directory for codex is the private `~/.codex/skills/`; `.agents` serves only as a read-only shared import source (source label `agents-shared`) — MAM scans it into the repository, with no tool attribution, no linking, and never writes to it (sole exception: one-time migration of MAM-created legacy links).

---

## Roadmap

- [x] US1 — Multi-tool session monitoring dashboard
- [x] US2 — Status change notifications & sound alerts
- [x] US3 — Quick terminal jump (iTerm2/Terminal.app/tmux)
- [x] US4 — Skill/MCP/Plugin unified repository management
- [x] US5 — Preset group one-click switching
- [x] US6 — Sub-agent level resource allocation
- [x] Resource dashboard redesign (dual-view + import + compatibility)
- [x] OpenClaw support (4th tool)
- [x] Kimi Code support (5th tool: session monitoring + MCP management + `KIMI_CODE_HOME` data directory redirection)
- [x] WorkBuddy support (6th tool: heartbeat-driven monitoring + deep-link jumps + resource management)
- [x] ZCode support (7th tool: SQLite session-aggregate monitoring + subagent-activity arbitration + workspace deep-link jumps + skill/MCP resource management)
- [x] dsh support (8th tool: web-host monitoring + zstd multi-frame log parsing + tri-color status + browser-tab jumps + read-only skill access)
- [x] Foxbell desktop pet (status cards + voice alerts + drag physics)
- [x] External pets (local/Petdex import + manage panel hot swap + capability gating)
- [x] Tool toggle management (batch save + restore/rebuild + full hiding)
- [x] APP-form session cards and deep-link jumps
- [x] Plugin management (file/config hybrid)
- [x] i18n (Chinese + English)
- [x] Auto-update via GitHub Releases
- [x] Dark/light theme sync with system
- [x] Windows support (NSIS installer + deep links + nearest-ancestor window focus)
- [ ] Linux support
- [ ] Kitty & WezTerm terminal jump support

---

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

1. Fork the repository
2. Create your feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'feat: add amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

Please read [AGENTS.md](AGENTS.md) for project architecture and development guidelines.

---

## License

This project is licensed under the MIT License — see the [LICENSE](LICENSE) file for details.

---

## Trademarks & Non-Affiliation

MultiAgents-Manager is an independent, open-source project. It is not affiliated with, endorsed by, or sponsored by Anthropic (Claude / Claude Code), OpenAI (Codex / ChatGPT), OpenCode, OpenClaw, Moonshot AI (Kimi Code), WorkBuddy, ZCode, dsh, or any other company or product mentioned in this repository. All product names, logos, and brands are the property of their respective owners; they are used here solely to describe compatibility (nominative fair use). Icons in this app are original designs; some color schemes are used only to help identify the corresponding tool and do not imply any official status.

## Official Channels

The only official distribution channel for this project is the [GitHub Releases](../../releases) page of this repository. Downloads offered anywhere else are third-party redistribution. See [TRADEMARK.md](TRADEMARK.md) for brand usage guidelines.
