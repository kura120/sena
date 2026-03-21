# ADR: Migrate UI from Freya to Tauri v2

## Status
Accepted

## Date
2026-03-21

## Context
The Sena PRD §13.1 specifies Freya (Rust-native, Skia backend) for the UI layer. During implementation of the debug overlay (PRD §15 Phase 1), we discovered Freya lacks critical capabilities needed for the Xbox Game Bar-style overlay design:

- **No native overlay/always-on-top support** — Freya windows cannot be set as always-on-top transparently
- **No click-through transparency** — Cannot pass clicks through empty regions to the desktop
- **No system tray integration** — No built-in support for tray icons or hiding from taskbar
- **No multi-window management** — Cannot create independent floating panels that move independently
- **Limited platform integration** — No access to platform-specific window effects (acrylic blur, etc.)

Tauri v2 provides all of these capabilities through its multi-window API, platform-specific window effects, global shortcut registration, and system tray support.

## Decision
Replace the Freya-based `ui/` crate with a Tauri v2 application:
- **Backend**: Rust (src-tauri/) — preserves all gRPC integration logic
- **Frontend**: React + TypeScript + Vite + Tailwind CSS
- **Package manager**: pnpm

All existing gRPC logic (event streaming from daemon-bus, chat via reactive-loop, signal mapping, boot timeline tracking) is ported to the Tauri Rust backend as Tauri commands and event emitters. The frontend receives data via Tauri's event system.

## Consequences
### Positive
- Full overlay capability: always-on-top, transparent, click-through, system tray
- Multi-window support: each panel is an independent window
- Global hotkey registration (configurable, default F12)
- Acrylic blur and other platform-specific window effects
- Cross-platform from day one (Windows, macOS, Linux)
- Rich frontend ecosystem (React, TypeScript, Tailwind)
- WebView2 on Windows, WebKit on macOS/Linux — no bundled browser engine

### Negative
- Introduces JavaScript/TypeScript to the stack (previously Rust-only UI)
- WebView has higher baseline memory than Skia-rendered Freya
- Additional build toolchain: Node.js + pnpm alongside Rust
- Deviation from PRD specification — must be documented

### Mitigations
- UI remains a thin rendering layer — all business logic stays in Rust backend
- gRPC client code is identical Rust (tonic) — no logic lost in migration
- TypeScript strict mode + ESLint enforce code quality on frontend
- Tauri v2's IPC is type-safe via command system

## Subsystems Affected
- `ui/` — complete replacement
- `daemon-bus/config/daemon-bus.toml` — supervisor command path update
- `.github/instructions/ui.instructions.md` — update to reflect Tauri architecture
