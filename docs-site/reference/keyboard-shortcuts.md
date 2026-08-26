---
title: PortBay Keyboard Shortcuts — Full Shortcut Reference
description: "Every keyboard shortcut in PortBay: command palette, project table navigation, start/stop/restart keys, log viewer search, and save — scoped by context."
---

# Keyboard Shortcuts

| Shortcut | Scope | Action |
| --- | --- | --- |
| `Cmd+K` / `Ctrl+K` | Global | Open the command palette. |
| `Cmd+N` | Command palette | Add a project. |
| `Shift+Cmd+.` / `Shift+Ctrl+.` | Global | Enter Stop All confirmation. |
| `Escape` | Modals and overlays | Close when safe. |
| `Cmd+S` / `Ctrl+S` | Project detail | Save project edits. |
| `ArrowDown` / `ArrowUp` | Projects table | Move the selected project row. |
| `S` | Projects table | Start the selected project. |
| `X` | Projects table | Stop the selected project. |
| `R` | Projects table | Restart the selected project. |
| `Enter` | Projects table | Open the selected project detail panel. |
| `/` | Log viewer | Focus log search. |
| `N` / `Shift+N` | Log viewer | Move to next or previous match. |

Shortcuts are intentionally sparse. PortBay favors visible controls for destructive or uncommon actions, with the command palette covering experienced-user workflows.

## Workflow canvas

The [workflow builder](/guides/workflows) is the one surface with a dense
shortcut set, because it is a canvas. Press `?` inside it for the same list in
the app.

| Shortcut | Action |
| --- | --- |
| `Cmd+K` | Command palette |
| `Cmd+Z` / `Cmd+Shift+Z` | Undo / redo (`Cmd+Y` also redoes) |
| `Cmd+C` / `Cmd+V` | Copy / paste selection |
| `Cmd+D` | Duplicate selection |
| `Backspace` | Delete selection |
| `Arrow keys` | Nudge selection 1 px |
| `Shift+arrows` | Nudge selection 10 px |
| `1` | Zoom to fit |
| `F` | Fit to selection |
| `Space` (hold) | Pan the canvas |
| `Shift+drag` | Marquee-select nodes |
| `Alt+drag` | Drag a duplicate off a node |
| Double-click canvas | Node search |
| Double-click title | Rename the workflow |
| Right-click | Node / canvas context menu |
| `Enter` | Generate (in the AI start modal) |
| `Escape` | Close menus and panels |
| `?` | Cheat sheet |

Dragging from a node's handle draws a connection — dropping it on a node
connects it. Dragging an existing edge's endpoint re-routes it; dropping it on
empty canvas deletes it.

## Capture library

| Shortcut | Action |
| --- | --- |
| `Arrow keys` | Move through the thumbnail grid |
| `Space` | Full-size preview |
| `Left` / `Right` (in preview) | Walk the library |
| `Enter` | Send the capture back to the Quick Access overlay |
| `Backspace` | Delete (moves the file to the Trash) |

See [Screen Capture](/guides/capture).
