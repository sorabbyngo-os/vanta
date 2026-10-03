<p align="center">
  <img src="assets/vanta-logo.png" alt="Vanta" width="420">
</p>

<h1 align="center">Vanta</h1>

<p align="center">
  <b>AI-native spatial computing environment for Sorabbyngo OS</b>
</p>

<p align="center">
  <img alt="Status: pre-alpha" src="https://img.shields.io/badge/status-pre--alpha-orange">
  <img alt="Language: Rust" src="https://img.shields.io/badge/language-Rust-b7410e">
  <img alt="Display protocol: Wayland" src="https://img.shields.io/badge/display-Wayland-4c7dff">
  <img alt="Part of Sorabbyngo OS" src="https://img.shields.io/badge/part%20of-Sorabbyngo%20OS-111827">
</p>

> **Status: pre-alpha, in design.** Vanta is not runnable yet. This README describes what we are building and how. The [roadmap](#roadmap) shows where we are, and the full plan is in [docs/Vanta-Vision-and-Plan.pdf](docs/Vanta-Vision-and-Plan.pdf).

## What is Vanta

Vanta is the spatial and interactive environment of Sorabbyngo OS. It is not designed as a traditional Linux desktop. It is the human-facing layer through which people see, touch and direct the **intelligent objects, spaces, agents and capabilities** that the Sorabbyngo platform provides.

It combines a Wayland compositor with an AI-native interaction model and the **ObjectSpace** runtime. Users work with objects and spaces instead of applications and files, and every existing Linux application keeps working.

> **ObjectSpace defines the world. Vanta is how the user experiences and manipulates that world.**

## The idea

Traditional desktops are organized around applications:

```text
User → Application → File → Operating System
```

Vanta changes the model. The user states what they want, or manipulates objects directly, and applications become implementation details.

- A document is not merely a file.
- A workspace is not merely a collection of windows.
- An application is not something the user needs to think about.

Two paths lead into the system, and Vanta is where both begin and end:

```text
Direct path:  User → Vanta → ObjectSpace → Vanta renders the result
Intent path:  User → Vanta → AION → ObjectSpace → Vanta renders the result
```

The direct path always works. The intent path goes through **AION**, the intelligence layer of Sorabbyngo OS, and is optional.

## Principles

**What Vanta is**

| Principle | In practice |
|---|---|
| Intent-first, never intent-only | Intent is the default way to work, and the direct path (mouse, keyboard, touch) always stays available |
| Objects instead of files | Information is an intelligent, contextual object provided by ObjectSpace |
| Spaces instead of windows | Work lives in persistent spaces and contexts |
| AI-native | AI is part of the environment, not only an app inside it |
| Spatial interaction | Objects and spaces can be shown and manipulated spatially (later phases) |
| Context-aware | Relationships between objects, tasks, spaces and sessions are preserved |
| Local-first | Core functionality works without a permanent cloud connection |
| Privacy-first | User data, context and interactions stay under the user's control |
| Linux-compatible | Existing Linux software keeps working |

**Engineering rules that make it reliable**

1. **The direct path always works.** Unplug the AI and the desktop must still feel complete.
2. **The compositor never waits on a model.** Windows appear instantly. AI-informed placement arrives afterward as an animation.
3. **One door for every action.** Shortcuts, the GUI and AION use the same typed action API. The AI has no private powers.
4. **Predictable and undoable.** Every AI action is visible and can be undone. Destructive actions ask first.
5. **No config files required.** Every setting has a GUI and can be changed by asking.
6. **Light by default.** Small footprint, quick start, and spatial features cost nothing when unused.
7. **Simple for non-technical users.** Familiar behavior first, with power features discoverable but never required.

## Architecture

| Layer | Role |
|---|---|
| **Vanta** | Compositor, window system, shell, settings, intent client, rendering and animation |
| **AION** | Intelligence layer: interprets intent and plans workflows |
| **ObjectSpace** | Objects, spaces, relationships, context, memory, sessions, permissions, synchronization |
| Platform services | Byngox, Byngonet, HAL, CML, Sentinel |
| Core and kernel | Sorabbyngo Core and the Sorabbyngo OS kernel |
| Linux and hardware | Linux compatibility layer, drivers, hardware |

Vanta is a set of cooperating processes in one Rust workspace:

| Component | Role |
|---|---|
| `vanta-comp` | Wayland compositor: window management, tiling and floating, animations, rendering, XWayland |
| `vanta-shell` | Panel, launcher, dock, notifications, lock screen, overview |
| `vanta-settings` | Settings app and the daemon that stores and applies settings live |
| `vanta-intent` | Client for AION: voice and text capture, feedback, confirmations, undo, and a minimal local fallback for core commands |
| `vanta-objects` | Bridge to ObjectSpace: projects objects and spaces into windows and scenes, and wraps legacy apps as objects |
| `vanta-ipc` | The shared typed action API and message types |

**Windows are one projection of an object.** A legacy Linux application is wrapped as an object, so it can belong to a space while still appearing as an ordinary window. Details are in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Motion

Motion is part of how the computer communicates, not decoration. Vanta aims for smooth transitions, spring and Bézier animation curves with per-event settings, dynamic layout changes and intelligent object placement, with effects budgeted so they don't quietly drain the battery.

## Technology (planned)

- **Rust** for the compositor, shell, daemons and IPC
- **[Smithay](https://github.com/Smithay/smithay)** as the Wayland compositor foundation, with XWayland for X11 apps
- **C and C++** at the edges, such as local AI engines reached through bindings
- **AION and ObjectSpace** reached through stable interfaces, stubbed in early phases

## Roadmap

- [x] Name, identity and repository
- [ ] **Phase 0:** prototype the feel on Hyprland with a basic shell
- [ ] **Phase 1:** compositor skeleton on Smithay, running nested in a window
- [ ] **Phase 2:** tiling, floating, snapping, workspaces and the animation engine
- [ ] **Phase 3:** shell with panel, launcher, notifications and lock screen
- [ ] **Phase 4:** action API, settings daemon and settings app
- [ ] **Phase 5:** intent path with AION, text first, then voice
- [ ] **Phase 6:** ObjectSpace integration: objects and spaces as windows
- [ ] **Phase 7:** installable Sorabbyngo OS session
- [ ] **Phase 8:** spatial computing: gestures, projection, cross-device spaces

## Repository layout (planned)

```text
assets/      logos and brand files
docs/        architecture, UX principles, the full plan (PDF)
crates/
  vanta-comp/       compositor
  vanta-shell/      panel, launcher, notifications
  vanta-settings/   settings app and daemon
  vanta-intent/     AION client, voice and text capture
  vanta-objects/    ObjectSpace bridge
  vanta-ipc/        shared action API
```

## Developing

Vanta can't be run yet. When the first crates land, development will need Linux with a Wayland-capable graphics stack and a recent stable Rust toolchain (via [rustup](https://rustup.rs)). Build and run instructions will be added here as soon as there is something to run.

## Part of Sorabbyngo OS

Vanta is one component of the Sorabbyngo OS ecosystem. See the [sorabbyngo-os](https://github.com/sorabbyngo-os) organization for the rest.

## License

To be decided. Until a license is added, all rights are reserved.

---

<p align="center"><i>Sorabbyngo. The Rhythm of Digital Life.</i></p>
