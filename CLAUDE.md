# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Dev Commands

```bash
pnpm install            # Install frontend dependencies
pnpm tauri:dev          # Start dev mode (Rust + Vite HMR)
pnpm build              # TypeScript compile + Vite bundle
pnpm check              # Full check: format + lint + build
pnpm format             # Prettier auto-format
pnpm format:check       # Prettier dry-run
pnpm lint               # ESLint check
pnpm lint:fix           # ESLint auto-fix

# Rust-only checks (from src-tauri/)
cd src-tauri && cargo check    # Compile-check Rust (fast)
cd src-tauri && cargo test     # Run Rust unit tests
cd src-tauri && cargo clippy   # Lint Rust code
```

There are no frontend test runner or e2e tests configured. Rust tests are inline `#[cfg(test)]` modules in `linker/layer2.rs`, `linker/layer3.rs`, and `adapter/mod.rs`.

## Architecture Overview

**Tauri 2 desktop app** with Rust backend + React 19 frontend, communicating via Tauri IPC (`invoke`).

### Backend (Rust) — `src-tauri/src/`

The backend follows a **layered architecture** with clear separation:

- **`adapter/`** — Tool-specific adapters implementing the `AgentAdapter` trait. Each adapter knows how to discover processes, parse sessions, read/write configs, and manage skill directories for one tool (Claude, Codex, OpenCode, OpenClaw). The trait is in `mod.rs`.
- **`monitor/`** — Process scanning (`process.rs`), JSONL session parsing (`parser.rs`), hook event reading (`hooks.rs`), and status determination (`status.rs`). OpenClaw uses a separate `openclaw_parser.rs` based on `state.json`.
- **`manager/`** — Business logic: skill install/enable/disable (`mod.rs`), MCP config write in JSON/TOML/JSONC (`mcp.rs`), preset apply/deactivate with compatibility checks (`preset.rs`), and plugin management (`plugin.rs`).
- **`linker/`** — Symlink/Junction management with three layers:
  - **Layer 1**: SSOT at `~/.mam/skills/` (global repo)
  - **Layer 2**: Tool-level active dir at `~/.mam/active/<tool>/skills/` (`layer2.rs`)
  - **Layer 3**: Sub-agent-level active dir at `~/.mam/active/<tool>/<sub-agent>/skills/` (`layer3.rs`)
- **`store.rs`** — SQLite data layer (`~/.mam/mam.db`). Tables: `session_status_cache`, `settings`, `extensions`, `extension_assignments`, `agent_tools`, `sub_agents`, `presets`, `preset_items`, `native_extensions`. Uses `once_cell::Lazy` + `Mutex<Connection>`.
- **`commands.rs`** — All `#[tauri::command]` IPC handlers. Registered in `lib.rs`.
- **`terminal/`** — Terminal focus via AppleScript (iTerm2/Terminal.app) and tmux.
- **`plugins/system_tray.rs`** — System tray with status indicator and preset menu.

**Key pattern**: All state is global (SQLite + `DashMap` for session cache). There is no per-request state or Tauri managed state — commands call module-level functions directly.

### Frontend (React/TypeScript) — `src/`

- **`pages/`** — Route pages: `home.tsx`, `settings.tsx`, `about.tsx`. Routing is path-based via `window.location.pathname`.
- **`components/`** — UI components. Key ones:
  - `SessionGrid.tsx` + `SessionCard.tsx` — Dashboard session cards
  - `ExtensionList.tsx` — Main resource view with dual-view switching (byKind/byTool)
  - `ResourceByKindView.tsx` — Skills/MCP/Plugins three-section view
  - `ResourceByToolView.tsx` — Four-tool card view (Claude/Codex/OpenCode/OpenClaw)
  - `PresetList.tsx` — Preset management with sub-agent actions
  - `McpManager.tsx` — MCP server add/edit/delete panel
  - `ImportDialog.tsx` + `CompatibilityDialog.tsx` — Resource import dialogs
  - `ui/` — shadcn/ui primitives (Radix UI based)
- **`stores/sessionStore.ts`** — Zustand store for session data (the only store)
- **`hooks/`** — `useSessions.ts` (polling), `useNotification.ts`, `useUpdater.ts`
- **`lib/`** — Utility modules: `audio.ts` (Web Audio chimes), `shortcut.ts`, `screenshot.ts`
- **`i18n/`** — i18next with Chinese (`zh`) and English (`en`) locales
- **`tauri-mock.ts`** — Mocks `__TAURI_INTERNALS__` for browser/Playwright rendering (auto-loads when `window.__TAURI_INTERNALS__` is absent)

### IPC Data Flow

```
Frontend (invoke) → commands.rs → manager/linker/adapter → store.rs (SQLite)
                                       ↓
                                  File system (symlinks, config files)
```

All `invoke` calls are synchronous on the Rust side (not async commands). The frontend polls sessions every 1.5s via `setInterval`.

### Three-Layer Skill Mapping

```
Layer 1 (SSOT):    ~/.mam/skills/brainstorming/SKILL.md
                         ↓ symlink
Layer 2 (Tool):    ~/.mam/active/claude/skills/brainstorming → Layer 1
                         ↓ symlink
Layer 3 (SubAgent):~/.mam/active/claude/sub-agent-1/skills/brainstorming → Layer 2
```

Layer 3 symlinks point to Layer 2 (not Layer 1), so disabling a tool-level link automatically breaks all sub-agent links. `cleanup_layer3_on_tool_disable` in `layer3.rs` handles cascade.

### Agent Adapter Pattern

Each tool implements `AgentAdapter` trait with these key methods:
- `find_processes()` — Scan running processes via `sysinfo`
- `find_sessions()` — Parse tool-specific session files (JSONL for Claude/Codex, SQLite for OpenCode, state.json for OpenClaw)
- `mcp_format()` / `mcp_config_path()` — How to write MCP config (JSON/TOML/JSONC)
- `skill_dirs()` — Where the tool reads skills from
- `hook_supported()` — Whether the tool supports status hooks

Adding a new tool: implement `AgentAdapter`, register in `get_all_adapters()` in `adapter/mod.rs`, add `detect_tools` entry in `commands.rs`.

## Data Directory

All app data lives in `~/.mam/`:

| Path | Purpose |
|------|---------|
| `~/.mam/mam.db` | SQLite database |
| `~/.mam/skills/` | Layer 1 global skill repo (SSOT) |
| `~/.mam/mcp/` | Global MCP server configs |
| `~/.mam/active/<tool>/` | Layer 2 tool-level active dirs |
| `~/.mam/active/<tool>/<sub>/` | Layer 3 sub-agent active dirs |
| `~/.mam/hooks/` | Status hook scripts |
| `~/.mam/events/` | Hook event files (auto-cleaned, 30s TTL) |
| `~/.mam/screenshots/` | Captured screenshots |

## Tool Config Locations

| Tool | Skills Dir | MCP Config | MCP Format |
|------|-----------|------------|------------|
| Claude Code | `~/.claude/skills/` | `~/.claude.json` | JSON |
| Codex CLI | `~/.agents/skills/` | `~/.codex/config.toml` | TOML |
| OpenCode | `~/.config/opencode/skills/` | `~/.config/opencode/opencode.json` | JSONC |
| OpenClaw | `~/.openclaw/skills/` | N/A | N/A |

## Language & Conventions

- **Design docs, specs, plans, and task lists** are written in **Chinese** (中文)
- **Code identifiers** use English; **code comments** prefer Chinese where possible
- **Git commit messages** may use English
- **Serialization**: Rust structs use `#[serde(rename_all = "camelCase")]` for JSON IPC. TypeScript types in `src/types/` mirror the camelCase output.
- **i18n**: All user-visible strings use `t()` from `react-i18next`. Locale keys are in `src/i18n/locales/`.
- **UI**: shadcn/ui components with Tailwind CSS v4. Use existing `ui/` primitives before adding new ones.
- **Window**: Main window is frameless (`decorations: false`, `transparent: true`). Title bar is custom (`title-bar.tsx`, `window-frame.tsx`).
- **Path aliases**: `@/` maps to `src/` (configured in `vite.config.ts` and `tsconfig.json`).

## Project Constitution (Key Rules)

Full constitution is at `.specify/memory/constitution.md`. Critical rules:

1. **Unified tech stack**: Rust backend + React frontend only. No Electron, Swift, Python, or Go in the core app.
2. **Adapter pattern**: New tools must only implement `AgentAdapter` — never modify core logic. Tool-specific code stays in `adapter/<tool>.rs`.
3. **Hook event case**: Claude Code uses PascalCase (`PreToolUse`), Codex uses camelCase (`preToolUse`). Adapters declare case via `hook_event_case()`.
4. **Non-invasive monitoring**: No background daemons (except the app itself), no external API calls, fully offline.
5. **Security**: Never read or expose sensitive paths (`~/.ssh`, `~/.gnupg`, `~/.aws`, `~/.kube`, `~/.netrc`, etc.). All skill/MCP installations must be visible and toggleable — never silent.
6. **Windows support**: Directory Junction (`mklink /J`) + file copy mode on Windows; do not rely on symlinks (requires admin).
