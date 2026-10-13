<p align="center">
  <img src="assets/vanta-logo.png" alt="Vanta" width="420">
</p>

<h1 align="center">Vanta</h1>

<p align="center">
  <b>The spatial desktop of Sorabbyngo OS</b><br>
  <i>Where intent meets the screen — and the direct path never dies.</i>
</p>

<p align="center">
  <img alt="Status: pre-alpha" src="https://img.shields.io/badge/status-pre--alpha-orange">
  <img alt="Language: Rust" src="https://img.shields.io/badge/language-Rust-b7410e">
  <img alt="Display: Wayland" src="https://img.shields.io/badge/display-Wayland-4c7dff">
  <img alt="Foundation: Smithay" src="https://img.shields.io/badge/foundation-Smithay-5b21b6">
</p>

---

## Vision

Most desktops are filing cabinets with windows.

Vanta is the **presence** of the machine: a spatial Wayland environment where work lives in **spaces**, meaning lives in **objects**, and intelligence is invited — never required.

You should be able to:

- Point, type, and tile as on any serious Linux desktop  
- Speak or write intent and watch the world rearrange through **AION**  
- Continue a space across devices without thinking about IPs or SSH  
- Run every normal Linux app while the native model stays objects and spaces  

Inspiration is simple: an OS that feels continuous — closer to a companion environment than a launcher grid — while remaining a real, inspectable, Linux-compatible system.

> **ObjectSpace defines the world.  
> AION decides and mediates.  
> Vanta is how you see and touch it.  
> Chameha is how you speak with it.**

---

## Interaction model

```text
Traditional:  User → Application → File → OS

Sorabbyngo:   User → Intent or Direct action
                ↓
              Vanta (this project)
                ├── direct: input → layout → pixels
                └── intent: Chameha → AION → ObjectSpace → Vanta renders
Path
Behavior
Direct
Mouse, keyboard, touch, shortcuts. Always complete without AI.
Intent
Voice/text via Chameha → AION plans → mediated actions → Vanta updates.
Rules that do not bend:
The compositor never blocks on a model.
AI has no private powers — same typed action API as the GUI.
Everything destructive is visible and undoable where possible.
Spatial features cost nothing when unused.
Architecture
┌─────────────────────────────────────────────┐
│  Vanta                                      │
│  ┌─────────────┐  ┌──────────────────────┐  │
│  │ vanta-comp  │  │ vanta-shell          │  │
│  │ Wayland     │  │ panel, launcher,     │  │
│  │ compositor  │  │ notifications, lock  │  │
│  └──────┬──────┘  └──────────┬───────────┘  │
│         │     vanta-ipc      │              │
│  ┌──────┴────────────────────┴───────────┐  │
│  │ vanta-intent │ vanta-objects │ settings│  │
│  │ (→ AION)     │ (→ ObjectSpace)         │  │
│  └──────────────┴─────────────────────────┘  │
│         Chameha lives here (voice / persona)  │
└─────────────────────────────────────────────┘
         │                    │
         ▼                    ▼
      AION                 ObjectSpace
   (control plane)      (objects & spaces)
Crate (planned)
Responsibility
vanta-comp
Smithay-based compositor: surfaces, seats, outputs, tiling/floating, animation
vanta-shell
Panel, launcher, dock, overview, notifications, lock screen
vanta-ipc
Typed action API shared by GUI, shortcuts, and AION
vanta-settings
Live settings daemon + UI
vanta-intent
Bridge to AION; text first, voice via Chameha
vanta-objects
Project ObjectSpace into windows/scenes; wrap X11/Wayland apps as objects
Chameha is not a separate desktop. It is the conversational face inside Vanta.
AION is not embedded as a second compositor brain. It is the system control plane Vanta calls.
Technical foundation
Layer
Choice
Why
Language
Rust
Safety, performance, Smithay ecosystem
Compositor
Smithay
Serious Wayland server toolkit in Rust
Protocol
Wayland + XWayland
Modern path + legacy apps
Session
systemd / logind (Fedora bootc base)
Seats, multi-seat, idle
IPC
Typed messages over local transport
One door for all actions
AI
Client only (AION)
Compositor stays responsive
Development target for early phases: nested Smithay session inside an existing Wayland (or X11) desktop — no custom kernel, no full OS image required.
What you can build on a modest machine
Vanta is the right place to work before the home lab:
Work
Needs full OS image?
Needs heavy GPU compile?
Repo layout, IPC types, docs
No
No
Smithay “hello compositor” nested window
No
Low–medium (Rust)
Tiling / floating logic, unit tests
No
Low
Shell UI prototypes (egui/iced later)
No
Low
bootc image, NVIDIA stack
Yes
Lab
Phase focus on this PC: scaffolding + nested compositor + action API stubs.
Roadmap
[x] Identity, vision, repository
[ ] Phase 0 — Workspace layout, vanta-ipc types, architecture docs
[ ] Phase 1 — Smithay nested compositor skeleton (vanta-comp)
[ ] Phase 2 — Tiling, floating, workspaces, basic animation
[ ] Phase 3 — Shell: panel, launcher, notifications
[ ] Phase 4 — Settings daemon + action API wired end-to-end
[ ] Phase 5 — Intent path (text → AION stub → confirm/undo)
[ ] Phase 6 — ObjectSpace bridge; apps as objects
[ ] Phase 7 — Installable session on Sorabbyngo/Fedora bootc
[ ] Phase 8 — Spatial / multi-device continuity (Byngonet)
Repository layout
vanta/
├── README.md
├── assets/
├── docs/
│   ├── ARCHITECTURE.md
│   └── PRINCIPLES.md
├── Cargo.toml                 # workspace
└── crates/
    ├── vanta-comp/
    ├── vanta-shell/
    ├── vanta-ipc/
    ├── vanta-settings/
    ├── vanta-intent/
    └── vanta-objects/
Developing
Today (design + skeleton): Linux, Rust via rustup, Wayland session recommended for nested testing.
# when crates exist
cargo build -p vanta-comp
cargo run -p vanta-comp    # nested compositor window
Full session integration targets the Fedora bootc–based Sorabbyngo image (see fedora-base).
Related
Project
Role
aion
AI runtime / control plane
objectspace
Objects and spaces
chameha
Voice and personality (in Vanta)
byngonet
Multi-device mesh
sorabbyngo-os
OS umbrella
fedora-base
Immutable base inheritance
License
To be decided. All rights reserved until a license is published.
�
Sorabbyngo
The Rhythm of Digital Life 
