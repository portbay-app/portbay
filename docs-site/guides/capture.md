---
title: Screen Capture — area, window, scrolling, and GIF
description: "PortBay's built-in screen capture: shortcuts, the Quick Access overlay, annotation and pinning, the capture library, and how captures feed the task board and workflows."
---

# Screen Capture

PortBay ships a screen-capture engine and a searchable **capture library**.
It takes area, window, fullscreen, scrolling, video and GIF captures, keeps a
history you can walk with the keyboard, and feeds captures straight into task
cards and workflows.

Open the library from **Capture** in the sidebar.

<ThemeImage name="capture" alt="The PortBay capture library — a filterable thumbnail grid of past captures" />

::: info macOS only
Capture rides a native macOS sidecar (`portbay-capture`). The sidebar entry is
hidden on other platforms.
:::

## Taking a capture

Capture types:

| Type | What it grabs |
|---|---|
| **Area** | A region you drag out. |
| **Window** | A single window, picked by hovering it. |
| **Fullscreen** | The whole display. |
| **Scrolling** | A long page, stitched as it scrolls. |
| **Recording** | A video of a region or window. |
| **GIF** | The same, as an animated GIF. |

Every capture lands on the **Quick Access overlay** — the floating thumbnail
that appears after the shot. From there you can drag it straight into any app,
pin it to the desktop, annotate it, or let it fall into the library.

### Permissions

Screen capture needs macOS **Screen Recording** permission. PortBay asks the
first time you capture; if you declined earlier, grant it in **System Settings →
Privacy & Security → Screen Recording** and use **Restart capture engine** in
the app.

### System shortcuts

PortBay can **take over the system screenshot shortcuts** (<kbd>⌘</kbd><kbd>⇧</kbd><kbd>3</kbd>/<kbd>4</kbd>/<kbd>5</kbd>)
so the keys you already use produce PortBay captures instead of loose files on
your Desktop. The takeover is opt-in and reversible — the status and the toggle
live on the Capture page.

## The library

The library is a thumbnail grid of everything you have captured, newest first,
filtered by the type chips along the top.

| | |
|---|---|
| Arrow keys | Move through the grid |
| <kbd>Space</kbd> | Full-size preview |
| <kbd>←</kbd> / <kbd>→</kbd> in preview | Walk the library |
| <kbd>↵</kbd> | Send the capture back to the Quick Access overlay |
| <kbd>⌫</kbd> | Delete |

**Enter** is the one worth remembering: it puts a capture from weeks ago back on
the overlay, where it can be dragged out, pinned, or re-annotated exactly like a
fresh one.

Per-item actions — **Pin**, **Annotate**, **Reveal in Finder**, **Copy** — act
on the saved file directly.

::: tip Delete is recoverable
Deleting an entry removes it from the library *and* moves the saved file to the
Trash. It is not a shred — you can get it back until you empty the Trash.
**Clear history** empties the whole library the same way.
:::

## Where captures go next

- **Task cards.** A capture — including a screen recording — can be attached to
  a card on the [task board](/guides/task-board). On Pro, attachments
  [sync to your other devices](/pro/#multi-device-sync), up to 250 MB.
- **Workflows.** The **Capture** node takes a capture as a step in a
  [workflow](/guides/workflows), so a run can screenshot what it just did.
- **Agents.** Over [MCP](/agents/), an agent can screenshot the page it is
  looking at and reason about the image.

## See also

- [Recordings](/guides/recordings) — replaying recorded SSH sessions
- [Task Board & Agents](/guides/task-board)
- [Workflows](/guides/workflows)
