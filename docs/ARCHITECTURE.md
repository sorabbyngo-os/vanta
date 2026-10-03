# Vanta architecture

> Draft. This describes the intended design. Nothing here is implemented yet, and details will change as we build. The full plan is in [Vanta-Vision-and-Plan.pdf](Vanta-Vision-and-Plan.pdf).

## Goals

Vanta is the human-facing environment of Sorabbyngo OS. The architecture protects three things:

1. **Responsiveness.** The frame loop never waits on anything slow, least of all an AI model.
2. **Resilience.** If AION is slow, offline or wrong, the desktop carries on.
3. **One consistent way to act.** The GUI, shortcuts and AION all act through the same door.

## Where Vanta sits

```text
YOU
 ↓
VANTA          compositor, window system, shell, settings, intent client
 ↓      ↘
 ↓       AION          intelligence layer (intent path, optional)
 ↓      ↙
OBJECTSPACE    objects, spaces, context, memory, sessions, permissions, sync
 ↓
Platform services · Sorabbyngo Core · Kernel · Linux
```

Two paths, and Vanta is where both begin and end:

```text
Direct path:  User → Vanta → ObjectSpace → Vanta renders the result
Intent path:  User → Vanta → AION → ObjectSpace → Vanta renders the result
```

## Who owns what

**ObjectSpace defines the world. Vanta is how the user experiences and manipulates it.**

| Concern | ObjectSpace | Vanta |
|---|---|---|
| Objects | Identity, state, permissions, history | Shows, selects, moves and opens them |
| Spaces | Which objects belong together, persistent sessions | Lays a space out on screen: tiling, floating, transitions |
| Context | Relationships and memory | Displays it, uses it for placement hints |
| Cross-device | Synchronization of objects, state and intent | Presents spaces across devices and displays |
| Rendering | Supplies the scene description | Renders it: compositor, effects, animation |
| Input | Receives the resulting actions | Captures keyboard, mouse, touch, voice, gestures |

This split is a proposal. The ObjectSpace documentation also lists workspace management, rendering, gesture and projection runtimes, so the exact contract has to be agreed with ObjectSpace and with the sorabbyngo-spatial work (canonical scene graph and rendering engine).

## Process model

Separate processes, so a failure in one cannot take down the others:

| Process | Responsibility | May block on AI? |
|---|---|---|
| `vanta-comp` | Wayland surfaces, input, tiling and floating, animations, rendering | **Never** |
| `vanta-shell` | Panel, launcher, dock, notifications, lock screen | No |
| `vanta-settings` | Settings app and the daemon that stores and applies settings | No |
| `vanta-intent` | AION client: voice and text capture, feedback, confirmations, undo, minimal local fallback | Yes, this is its job |
| `vanta-objects` | ObjectSpace bridge: projects objects and spaces into windows and scenes | No |

All of them talk through `vanta-ipc`. `vanta-intent` is a client of AION, not a second AI stack.

## The action API

Every operation is a typed **action**. Shortcuts, the GUI and AION all send the same actions.

Illustrative sketch (not final):

```rust
enum Action {
    PlaceObject { object: ObjectId, space: SpaceId, placement: Placement },
    SetLayout { space: SpaceId, layout: Layout },
    Focus { object: ObjectId },
    Close { object: ObjectId },                 // destructive
    SetSetting { key: SettingKey, value: Value },
    // ...
}
```

Rules:

- The API speaks in **objects and spaces**, not window IDs. A window is one projection of an object.
- The assistant has no private powers. If a user can't do it through an action, AION can't either.
- Actions are logged, so each one can be shown and undone.
- Destructive actions from AION require confirmation.

## Windows and legacy apps

To the compositor, a window is a surface with an application ID, a title and a size. Meaning, such as "this belongs with your project", lives above it in ObjectSpace and AION, and reaches the compositor as actions.

A legacy Linux application is wrapped as an object by `vanta-objects`, so it can belong to a space while still appearing as an ordinary window through Wayland or XWayland.

## Window placement

When an app opens, the compositor places the window immediately using fast rules: the current layout, focus and the user's saved habits. It never waits for a model. Afterward, AION may suggest a better position, and the window animates there. One gesture undoes it. Vanta learns from corrections and does not move windows without a clear reason.

## Settings

- Settings follow a schema and are stored by a daemon.
- Changes apply live, with no reload.
- The GUI and AION write settings through the action API.
- Files exist internally, but users are never required to edit them.

## Animations

Animation lives in the compositor's render loop, with spring and Bézier curves and per-event settings for windows, spaces and fades. Effects that force a frame on every refresh are used sparingly, because they cost battery.

## Offline behavior

Offline behavior is inherited from AION and ObjectSpace. Vanta's own requirement is that direct manipulation and core commands work with no connection, and that `vanta-intent` degrades to a minimal local fallback when AION is unavailable.

## Spatial computing (later)

Gestures, projection, shared spaces, cross-device spaces and immersive surfaces arrive in later phases. The first phases keep the action API and object model free of 2D-only assumptions, and the compositor is designed to consume the canonical scene graph and rendering engine once their contract is agreed.

## Open questions

- Which GUI toolkit for the shell and settings app?
- What is the exact contract with ObjectSpace and the sorabbyngo-spatial scene graph and rendering engine?
- Default layout for first-time users: floating with easy snapping, or tiling?
- How does Vanta relate to Chameha, which the ObjectSpace documentation shows above AION?
- Which local models and hardware floor for the offline tier?
- Which license for Vanta? It also decides what code can be reused from other compositors.
