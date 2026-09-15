# Preset Groups

Exclusive preset application, restore-to-default, the stash area, resident locks, exclusive bindings, and consistency health checks

## Overview

A preset group bundles a set of skills / MCP servers / plugins into a single "kit" that you can apply to an AI coding tool in one click. It solves a simple problem: different tasks call for different tool combinations (one set for front-end work, another for data processing), and toggling resources by hand is tedious and error-prone. Preset groups use **exclusive semantics** — once applied, a tool sees only the resources inside the preset plus whatever you marked as "resident"; everything else steps aside. Turning the preset off restores the tool to its pre-application state.

## Core Concepts

- **Exclusive kit**: applying a preset = "disable/stash what's currently active → enable the preset's items". After applying, the tool's resource surface = preset items + resident items, nothing else.
- **Universal presets**: contain only MAM repository resources (skills / MCP / plugins) and can be applied to any enabled tool.
- **Tool-scoped presets**: bound to a single tool; besides MAM resources they may also include that tool's **native skills** (skills installed directly in the tool's own directory, outside the MAM repository).
- **At most one active preset per tool**: switching on a new preset for a tool automatically switches the previous one off. With everything off, the tool is back to its original state — MAM takes a "base snapshot" before each application, and turning the preset off restores from that snapshot.

## The Preset List

The preset section of the Resources page is split into **two sections**:

- **Universal presets**: not bound to any tool; each card carries one toggle switch per enabled tool.
- **Tool-scoped presets**: grouped by their bound tool (group headers show the tool icon); each card has a single switch for the bound tool.

Each preset card shows: the preset name, a one-line truncated description, an items-count badge (e.g. `3`), a toggle switch per usable tool, and a delete button. Clicking anywhere else on the card opens the edit dialog.

**Deletion protection**: while a preset is active on some tool, deleting it is refused with "This preset group is currently applied and cannot be deleted; turn it off first" (saving edits is blocked the same way). Turn the preset off on that tool first, then delete.

## Apply and Restore (Toggle Switches)

- **On = apply**: turning a switch on first opens a **confirmation dialog** showing a dry-run preview of the full diff, in five sections (empty sections are hidden):
  1. **To enable** — items that will actually be enabled after exclusive-binding filtering;
  2. **Filtered out** — items skipped because their exclusive binding does not allow this tool, each with the reason;
  3. **To disable** — MAM resources that will be swept away;
  4. **To stash** — native skills that will be moved into the stash area (named one by one);
  5. **Resident exempt** — resources outside the preset that the resident lock protects from the sweep (count only).

  Execution starts only after you confirm; a toast then reports "applied N, disabled N, stashed N".
- **Off = restore default**: executes immediately (no confirmation dialog), restoring the tool to its pre-application state from the base snapshot; a toast reports what was restored, and any conflicts are listed in a warning toast.
- **Whiteboard mode**: if the preset has nothing to enable at all (for instance, one created just to "clear the stage"), the confirmation dialog shows a prominent warning at the top — "Whiteboard mode: this application will not enable any resources; it only disables/stashes existing resources (resident items exempt)" — and the confirm button turns into a red destructive style. You can still proceed; the point is that the consequence is spelled out.
- If a preset contains resource types the target tool does not support, the corresponding switch is disabled with a "not supported yet" hint. A switch that is already on, however, **can always be turned off** (off is the safe restore path).

## The Stash Area

When a preset is applied, the tool's **non-resident** native skill directories are moved wholesale into the stash area at `~/.mam/stash/<tool>/skills/` (a same-volume rename, zero copying); restoring the default moves them back. Every move is recorded in a ledger (stash_journal), so after a crash the next start can reconcile from it.

- **Same-name conflicts**: if the original spot is occupied when restoring (say you manually installed a same-named skill during the preset session), MAM **never overwrites** — the item stays in the stash area and is reported as a conflict for you to resolve manually.
- Stash entries that never got restored also show up in the health check card under "pending stash entries", each with a manual "restore" button.

## Resident Locks

Some resources should survive every preset — your core MCP servers, must-have skills. Each resource row on a tool carries a lock switch (the resident lock); once on, that resource is "resident" for the tool and the card shows a "resident" badge.

- The exclusive sweep **never touches** resident resources: they are neither disabled nor stashed.
- The "resident exempt" count in the apply confirmation dialog is exactly what this lock saved from the sweep.

## Exclusive Bindings

Some resources depend on a specific tool's environment (an app, a companion MCP); putting them into other tools only causes trouble. An exclusive binding restricts a resource to the tools you allow:

- An orange **"exclusive" badge** next to the resource name marks a bound resource; clicking it opens an editor where you can check the **allowed tools** and record an **exclusive reason** (say why — e.g. "depends on the codex app + computer-use MCP"). Saving with nothing checked clears the binding and makes the resource universal again.
- Toggle buttons on tools excluded by the binding are **greyed out**, with the reason on hover.
- The per-resource "enable all" bulk action **automatically skips** excluded tools; the "skipped" count in the completion toast comes from here.
- **Automatic SKILL.md detection**: if a skill's SKILL.md frontmatter declares its exclusive tools (recognized keys: `supported_agents` / `agents` / `compatible_tools` / `tools`; values may be a YAML array `["a", "b"]` or a comma-separated string `"a, b"`), importing it pops a confirmation asking whether to write the binding as declared; declarations on already-installed skills are listed in the health check card under "pending exclusive suggestions", each with "set exclusive" or "ignore this round". Detection is **suggestion only, never enforced** — manual bindings are always the source of truth.
- **Known limitation**: block-list style declarations are not recognized yet — i.e. `tools:` followed by indented `- item` lines (`tools:\n  - claude`) counts as "not declared". Use the array or comma-separated form instead.

## Consistency Health Checks

Preset exclusivity and enable/disable both rely on the ledger (database) and the disk (symlinks/directories) agreeing. Both the Resources page and the Settings page carry a **consistency health check** card: its badge shows the number of unresolved issues, and "Run now" rescans at any time. It aggregates four kinds of information:

**① Ledger–disk drift** (grouped by tool, four badges):

| Level | Name | Meaning |
|-------|------|---------|
| L1 | Missing link | The ledger says enabled, but the link is gone from disk |
| L2 | Real directory | A real directory sits where a link should be, and the ledger says enabled |
| L3 | Extra link | A link into the MAM repository exists on disk, but the ledger does not record it |
| L4 | External link | The link points outside `~/.mam` (not MAM's business) |

Each row offers three dispositions:

- **a. Trust the ledger, fix the disk** — L1 rebuilds the link; L2 replaces with a link after verifying contents match; L3 removes the link;
- **b. Trust the disk, write back the ledger** — accept what's on disk and update the ledger to match;
- **c. Leave it for now** — collapses the row for this round (it reappears on the next check).

Two safety floors: **if L2-a finds the real directory's contents differ from the shared repository copy, the row escalates to "needs manual attention" — MAM never deletes a real directory**; and L4 always escalates to manual regardless of choosing a or b, leaving the scene untouched. Group headers offer batch buttons ("fix all disk from ledger / write all back to ledger") that process a whole tool at once.

**② Snapshot invariant violations**: leftovers such as "snapshot exists but no active preset", each with a one-click fix (which runs restore-default for that tool).

**③ Pending stash entries**: unrestored stashed skills, each with a "restore" button.

**④ Pending exclusive suggestions**: see Exclusive Bindings above; suggestions are listed only, never written automatically.

## Tray Shortcuts

The system tray menu has a "Preset Groups" section for quick switching without opening the main window:

- Every preset item **shows its checked state** (checked while active on a tool).
- **Clicking executes directly** — no confirmation dialog: unchecked → apply; checked → restore default.
- **Universal presets** go through a submenu to pick the tool first; tool-scoped presets are top-level items that act on their bound tool directly.
- Success and failure are logged, and the menu is rebuilt afterwards to reflect the real state; on failure the checkmark returns to the true state so you can simply retry.

## Notes

- Applying or restoring presets really moves directories and rewrites tool configurations. **Back up `~/.mam/` before your first run or any manual testing**; you can also verify in an isolated environment (the `MAM_HOME` environment variable redirects the data directory, effective in development/debug builds only).
- Turning a preset off = restore default, and it is idempotent: with no active preset, running it changes nothing.
- Whiteboard mode only clears the stage and enables nothing — confirm only when that is what you want.

![Screenshot](../../assets/screenshots/en/06-presets.png)
