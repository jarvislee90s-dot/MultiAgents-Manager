<div align="center">

# MultiAgents Manager

**Unified Management Platform for Multi-Agent Programming Tools**

A desktop app to monitor, notify, jump to, and manage Claude Code / Codex CLI / OpenCode / OpenClaw sessions

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Tauri v2](https://img.shields.io/badge/Tauri-v2-blue?logo=tauri)](https://v2.tauri.app/)
[![React 19](https://img.shields.io/badge/React-19-61DAFB?logo=react)](https://react.dev/)

English · [中文](README.md)

</div>

---

## Features

### Session Monitoring Dashboard

Real-time traffic-light status board for all active AI coding tool sessions.

| Status | Meaning |
|--------|---------|
| 🔴 Red | Waiting for user input |
| 🟡 Yellow | Processing / Thinking |
| 🟢 Green | Idle / Finished |

- Auto-discovers running **Claude Code**, **Codex CLI/APP**, **OpenCode**, and **OpenClaw** sessions
- Distinguishes CLI vs. desktop APP form (APP shows status only, no terminal jump)
- Shows project name, git branch, last message preview, CPU usage, runtime
- Sorts by priority: waiting → running → idle
- System tray icon reflects aggregate status (🔴/🟡/🟢)

### Desktop Notifications & Sound Alerts

- Color-change-based notifications (red↔yellow↔green) with deduplication
- Web Audio API chimes — no audio files needed
- Configurable on/off toggle in settings
- Clickable notifications with "View Session" action to jump to terminal

### Quick Terminal Jump

Click a session card to instantly focus the corresponding terminal tab:

| Terminal | Support |
|----------|---------|
| iTerm2 | ✅ AppleScript |
| Terminal.app | ✅ AppleScript |
| tmux | ✅ pane selection + terminal focus |
| Wayland | ❌ Graceful fallback message |

### Extension Resource Management

Unified repository for Skills, MCP servers, and Plugins across tools:

- **Skills**: Symlink (Unix) / Junction (Windows) mapping to each tool's skill directory
- **MCP Servers**: Auto-format conversion — JSON (Claude) / TOML (Codex) / JSONC (OpenCode)
- **Plugins**: File/config hybrid management
- Auto-import existing skills on first launch (from `~/.claude/skills/`, `~/.agents/skills/`, `~/.config/opencode/skills/`)
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

---

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Desktop Framework | [Tauri v2](https://v2.tauri.app/) (Rust) |
| Frontend | [React 19](https://react.dev/) + [TypeScript](https://www.typescriptlang.org/) |
| UI Components | [shadcn/ui](https://ui.shadcn.com/) (Radix UI) |
| Styling | [Tailwind CSS v4](https://tailwindcss.com/) |
| State Management | [Zustand](https://zustand-demo.pmnd.rs/) |
| i18n | [i18next](https://www.i18next.com/) (Chinese / English) |
| Database | [SQLite](https://www.sqlite.org/) (via [rusqlite](https://github.com/rusqlite/rusqlite)) |
| Process Monitoring | [sysinfo](https://github.com/GuillaumeGomez/sysinfo) |

## Architecture

```
src-tauri/src/
├── adapter/           # Agent adapter trait + per-tool implementations
│   ├── claude.rs      #   Claude Code (JSONL + Hook)
│   ├── codex.rs       #   Codex CLI/APP (JSONL + Hook)
│   ├── opencode.rs    #   OpenCode (SQLite)
│   ├── openclaw.rs    #   OpenClaw (state.json)
│   └── mod.rs         #   AgentAdapter trait + session discovery scheduler
├── monitor/
│   ├── process.rs     #   Process discovery (sysinfo scan)
│   ├── parser.rs      #   Claude & Codex JSONL parser
│   ├── opencode_parser.rs # OpenCode SQLite parser
│   ├── openclaw_parser.rs # OpenClaw state.json parser
│   ├── status.rs      #   Pure-message status determination
│   └── hooks.rs       #   Hook registration + event file reader
├── manager/
│   ├── mod.rs         #   Skill install/enable/disable + auto-import
│   ├── mcp.rs         #   MCP config writer (JSON/TOML/JSONC)
│   ├── preset.rs      #   Preset apply/deactivate + compatibility check
│   └── plugin.rs      #   Plugin management
├── linker/
│   ├── mod.rs         #   Symlink/Junction management + security checks
│   ├── detector.rs    #   Tool installation detection
│   ├── layer2.rs      #   Layer 2 tool-level active directory
│   └── layer3.rs      #   Layer 3 sub-agent-level active directory
├── terminal/          #   Terminal focus (iTerm2/Terminal.app/tmux)
├── plugins/
│   └── system_tray.rs #   System tray with status + preset menu
├── store.rs           #   SQLite data layer
├── commands.rs        #   Tauri IPC commands
├── session/           #   Session model + status enum
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

| Path | Purpose |
|------|---------|
| `~/.mam/mam.db` | SQLite database (settings, extensions, presets, session cache) |
| `~/.mam/skills/` | Global skill repository |
| `~/.mam/mcp/` | Global MCP server configs |
| `~/.mam/hooks/status-hook.sh` | Shared Hook script for status events |
| `~/.mam/events/` | Hook event files (auto-cleaned, 30s TTL) |

### Supported Tool Configs

| Tool | Skill Directory | MCP Config | MCP Format | Hook Support |
|------|----------------|------------|------------|-------------|
| Claude Code | `~/.claude/skills/` | `~/.claude.json` | JSON | ✅ (PascalCase) |
| Codex CLI | `~/.agents/skills/` | `~/.codex/config.toml` | TOML | ✅ (camelCase) |
| OpenCode | `~/.config/opencode/skills/` | `~/.config/opencode/opencode.json` | JSONC | ❌ |
| OpenClaw | `~/.openclaw/skills/` | N/A | N/A | ❌ |

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
- [x] Plugin management (file/config hybrid)
- [x] i18n (Chinese + English)
- [x] Auto-update via GitHub Releases
- [x] Dark/light theme sync with system
- [ ] Linux & Windows support (currently macOS primary)
- [ ] Kitty & WezTerm terminal jump support

---

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

1. Fork the repository
2. Create your feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'feat: add amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

Please read [CLAUDE.md](CLAUDE.md) for project architecture and development guidelines.

---

## License

This project is licensed under the MIT License — see the [LICENSE](LICENSE) file for details.
