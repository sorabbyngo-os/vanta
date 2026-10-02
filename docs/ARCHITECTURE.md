# Vanta architecture

> Draft. This document describes the intended design. Nothing here is implemented yet, and the details will change as we build.

## Goals

Vanta is a complete Wayland desktop whose assistant can operate it, while the desktop stays fully usable without the assistant. The architecture exists to protect three things:

1. **Responsiveness.** The frame loop must never wait on anything slow, least of all an AI model.
2. **Resilience.** If the assistant crashes, is offline, or is wrong, the desktop carries on.
3. **One consistent way to act.** The GUI, the shortcuts and the assistant all do things through the same door.

## Process model

Separate processes, so a failure in one cannot take down the others:

| Process | Responsibility | May block on AI? |
|---|---|---|
| `vanta-comp` | Compositor: Wayland surfaces, input, tiling and floating layout, animations, rendering | **Never** |
| `vanta-shell` | Panel, launcher, dock, notifications, lock screen | No |
| `vanta-settings` | Settings app (GUI) and the daemon that stores and applies settings | No |
| `vanta-ai` | Voice and text input, intent understanding, local and online models | Yes, this is its job |

All of them talk through `vanta-ipc`.

## The action API

Every operation a user can perform is a typed **action**. Shortcuts, the GUI and the assistant all send the same actions to the compositor.

Illustrative sketch (not final):

```rust
enum Action {
    MoveWindow { window: WindowId, target: Placement },
    SetLayout { workspace: WorkspaceId, layout: Layout },
    FocusWindow { window: WindowId },
    CloseWindow { window: WindowId },          // destructive
    SetSetting { key: SettingKey, value: Value },
    // ...
}
```

Rules:

- The assistant has no private powers. If a user can't do it through an action, the assistant can't either.
- Actions are logged, so each one can be shown to the user and undone.
- Destructive actions (closing windows, deleting things) require confirmation when they come from the assistant.

## Two paths into the desktop

- **Direct path.** The user acts through the shell, settings or keyboard shortcuts. It does not touch the assistant, so it always works.
- **Intent path.** The user speaks or types a request. `vanta-ai` turns it into one or more actions and sends them through the same action API.

If the intent path fails, nothing else is affected.

## Window placement

When an app opens, the compositor places the window immediately using fast rules: the current layout, focus, and the user's saved habits. It never waits for a model.

After the window is placed, `vanta-ai` may suggest a better position, for example beside related work. If it does, the window animates there, and the user can undo that move with one gesture. Placement should feel predictable, so Vanta learns from corrections and doesn't move windows around without a clear reason.

## What the compositor knows

To the compositor, a window is a surface with an application ID, a title and a size. **Semantic ideas**, such as "this belongs with your coding work", live above the compositor in `vanta-ai`, which tells the compositor what to do through actions. This keeps the compositor small, fast and stable.

## Settings

- Settings have a typed schema, and a daemon stores them.
- Changes apply live. There is no reload step.
- The GUI and the assistant both write settings through the action API.
- Files exist internally, but users are never required to edit them.

## Animations

Animations are part of the compositor's render loop, with curves (such as spring and Bézier) and per-event settings for windows, workspaces and fades. Effects that need a new frame on every screen refresh are used sparingly, because they cost battery.

## Offline and online AI

`vanta-ai` has two tiers:

- **Local tier:** a small model for the core commands (opening things, moving windows, changing settings). It works with no connection.
- **Online tier:** larger models for harder requests, used only when available and only when the user allows it.

If the online tier is unavailable, the local tier handles what it can and says clearly when something needs a connection.

## Out of scope for now

Spatial, holographic and cross-device surfaces are future directions. We will keep the action API and object model extensible enough not to block them, but we are building the 2D desktop first.

## Open questions

- Which GUI toolkit for the shell and settings app?
- Which local models and sizes fit modest laptops?
- Default layout behavior for first-time users (floating with easy snapping, or tiling).
- Which license to use, which also decides what code can be reused from other projects.
