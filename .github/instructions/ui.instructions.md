---
applyTo: "ui/**"
---

# ui — Copilot Instructions

The UI subsystem is built with **Tauri v2** — a multi-window desktop app framework with a Rust backend and a React + TypeScript frontend. The Rust backend (`ui/src-tauri/`) owns all gRPC client connections to daemon-bus, event stream subscriptions, and state management. The React frontend (`ui/src/`) is a thin rendering layer that receives state via Tauri commands and events.

The UI's job is to render state and deliver user input events. It makes no decisions, holds no business logic, and never touches memory or agents directly.

These rules are traps specific to this subsystem. Global rules in `.github/copilot-instructions.md` also apply in full.

---

## Architecture Overview

- **Backend**: Rust (Tauri) in `ui/src-tauri/`
  - gRPC clients (tonic) connect to daemon-bus and reactive-loop
  - Event stream subscription runs in background tokio task
  - Tauri commands (`#[tauri::command]`) expose backend state to frontend
  - Tauri events (`app_handle.emit`) push updates to frontend in real-time
- **Frontend**: React + TypeScript + Vite + Tailwind CSS in `ui/src/`
  - React components render state received from backend
  - `invoke()` calls Tauri commands to fetch state or trigger actions
  - `useTauriEvent()` hook subscribes to backend events for reactive updates
  - No business logic — components are pure rendering only
- **Multi-window**: Each overlay panel is an independent Tauri window
  - Always-on-top, transparent, click-through capable
  - Per-window position saving via `save_window_position` command
  - Global hotkey (F12 default) toggles all panels
  - System tray for minimize/quit

---

## Ownership Boundaries

UI owns:
- All visual components and layout (React frontend)
- Receiving state updates from daemon-bus via gRPC (Rust backend)
- Sending user input events to daemon-bus/reactive-loop via gRPC (Rust backend)
- Multi-window overlay management (Tauri backend)
- System tray and global hotkey registration (Tauri backend)

UI does not own:
- Any business logic — the UI renders state, it never makes decisions
- Any memory reads or writes — memory-engine owns all memory operations
- Any agent calls — agents communicate through daemon-bus only
- Any SoulBox reads directly — SoulBox state arrives via daemon-bus events
- Any model calls — inference subsystem owns the model exclusively

If you find yourself writing logic that determines what Sena should do inside a UI component or Tauri command, stop. That logic belongs in an agent, CTP, or the relevant backend subsystem.

---

## Tauri Backend Traps (Rust in `ui/src-tauri/`)

### State Lives in the Rust Backend — Never in React
All authoritative state is held in `DebugState` (Rust, behind `Arc<Mutex<_>>`). React components query this state via Tauri commands or receive updates via Tauri events. Never store business state in React component state beyond what's needed for rendering.

```rust
// good — state in Rust backend
pub struct DebugState {
    pub subsystem_health: HashMap<String, SubsystemHealthEntry>,
    pub thought_feed: VecDeque<ThoughtEvent>,
    // ...
}

#[tauri::command]
pub fn get_debug_snapshot(state: tauri::State<AppState>) -> DebugSnapshot {
    let guard = state.debug_state.lock().unwrap();
    build_snapshot(&guard)
}
```

```typescript
// good — React fetches state from backend
const [snapshot, setSnapshot] = useState<DebugSnapshot | null>(null);
useEffect(() => {
  invoke<DebugSnapshot>("get_debug_snapshot").then(setSnapshot);
}, []);
```

### gRPC Connections Live in Tauri Backend Only
Never create gRPC client connections in the React frontend. All tonic client code lives in `ui/src-tauri/src/grpc.rs`. React communicates with gRPC services indirectly via Tauri commands that wrap the gRPC calls.

```rust
// good — gRPC client in Tauri backend
pub async fn send_chat_message(
    address: &str,
    content: String,
    timeout_ms: u64,
) -> Result<UserMessageResponse, tonic::Status> {
    let channel = Channel::from_shared(address.to_string())?
        .connect_timeout(Duration::from_millis(timeout_ms))
        .connect()
        .await?;
    let mut client = UserMessageServiceClient::new(channel);
    let response = client.send_message(UserMessageRequest { message: content }).await?;
    Ok(response.into_inner())
}

#[tauri::command]
pub async fn send_message(
    content: String,
    state: tauri::State<'_, AppState>,
) -> Result<SendMessageResponse, String> {
    send_chat_message(&state.config.grpc.reactive_loop_address, content, state.config.grpc.connection_timeout_ms)
        .await
        .map(|r| SendMessageResponse { /* map fields */ })
        .map_err(|e| format!("gRPC error: {}", e))
}
```

```typescript
// good — React invokes Tauri command
await invoke<SendMessageResponse>("send_message", { content: userInput });
```

### Event Streams Run as Background Tasks
The daemon-bus event stream subscription runs as a long-lived tokio task spawned in `main.rs` setup. It processes events, updates `DebugState`, and emits Tauri events to the frontend. Never poll for events — always stream.

```rust
// good — streaming event subscription
pub async fn run_event_stream(
    app_handle: tauri::AppHandle,
    address: String,
    connection_timeout_ms: u64,
    reconnect_config: ReconnectConfig,
    debug_state: Arc<Mutex<DebugState>>,
) {
    loop {
        match connect_and_stream(&app_handle, &address, connection_timeout_ms, &debug_state).await {
            Ok(_) => { /* clean disconnect */ }
            Err(e) => {
                error!(error = %e, "Event stream error");
                // exponential backoff reconnect
            }
        }
    }
}
```

### Tauri Commands Are the API Surface to React
Every operation the frontend needs from the backend is exposed as a `#[tauri::command]` function. Tag all command functions in `invoke_handler` in `main.rs`. Never expose internal Rust APIs directly to TypeScript.

```rust
#[tauri::command]
pub fn get_overlay_config(state: tauri::State<AppState>) -> OverlayConfigResponse {
    OverlayConfigResponse {
        toggle_key: state.config.overlay.toggle_key.clone(),
        panels: state.config.overlay.panels.iter().map(|p| /* map */).collect(),
    }
}

// in main.rs setup:
.invoke_handler(tauri::generate_handler![
    get_overlay_config,
    send_message,
    // ...
])
```

### No Blocking Calls in Async Tauri Commands
Tauri commands run on the tokio runtime. Never perform synchronous blocking I/O in a command function. Use `tokio::task::spawn_blocking` for CPU-bound or sync I/O work.

```rust
// bad — blocks tokio runtime
#[tauri::command]
pub fn parse_large_config() -> String {
    std::fs::read_to_string("large_file.toml").unwrap() // blocks
}

// good — offload to blocking thread
#[tauri::command]
pub async fn parse_large_config() -> Result<String, String> {
    tokio::task::spawn_blocking(|| {
        std::fs::read_to_string("large_file.toml")
    })
    .await
    .map_err(|e| format!("spawn error: {}", e))?
    .map_err(|e| format!("read error: {}", e))
}
```

---

## React Frontend Traps (TypeScript in `ui/src/`)

### Components Are Pure Rendering Only
React components receive props or fetch state via `invoke()` and render. They never contain business logic. If a component is making decisions about what Sena should do, that logic belongs in the Rust backend.

```tsx
// bad — business logic in component
function ThoughtStream() {
  const [thoughts, setThoughts] = useState([]);
  const filtered = thoughts.filter(t => 
    t.relevance_score > getThreshold() && shouldDisplay(t) // business logic
  );
  return <div>{filtered.map(renderThought)}</div>;
}

// good — component renders data as-is
function ThoughtStream() {
  const [thoughts, setThoughts] = useState([]);
  return <div>{thoughts.map(renderThought)}</div>;
}
```

### State Fetching: Command + Event Pattern
Components fetch initial state via `invoke()` and subscribe to updates via `useTauriEvent()`. This ensures components always reflect backend state without polling.

```tsx
function SubsystemHealth() {
  const [subsystems, setSubsystems] = useState<SubsystemData[]>([]);

  // Fetch initial state
  const fetchState = useCallback(() => {
    invoke<DebugSnapshot>("get_debug_snapshot").then((snapshot) => {
      setSubsystems(snapshot.subsystems);
    });
  }, []);

  useEffect(() => {
    fetchState();
    const interval = setInterval(fetchState, 2000); // periodic sync
    return () => clearInterval(interval);
  }, [fetchState]);

  // Subscribe to real-time updates
  useTauriEvent<SubsystemStatusPayload>("subsystem-status-updated", (event) => {
    setSubsystems(prev => prev.map(s => 
      s.name === event.subsystem ? { ...s, status: event.status } : s
    ));
  });

  return <div>{/* render */}</div>;
}
```

### Never Create gRPC Connections in React
All gRPC client code is in the Tauri Rust backend. React components call Tauri commands which internally make gRPC calls. Never import or use tonic or any gRPC libraries in TypeScript.

### Animations Use CSS Transitions or Tailwind
For simple transitions (opacity, transform), use CSS transitions or Tailwind utility classes. For complex animations, use a React animation library (framer-motion, react-spring). Never implement animations with manual setInterval loops.

```tsx
// good — CSS transition
<div className="transition-opacity duration-300 hover:opacity-80">
  {content}
</div>

// good — Tailwind transition utilities
<div className="transform transition-transform hover:scale-105">
  {content}
</div>
```

### No Hardcoded Strings in Components
All user-facing text comes from `src/constants/strings.ts`. Never write literal UI text in a component.

```tsx
// bad
<div>Debug Overlay</div>

// good
import { STRINGS } from "../../constants/strings";
<div>{STRINGS.PANEL_TITLE}</div>
```

---

## Debug Overlay Traps

### Each Panel Is an Independent Tauri Window
The debug overlay is composed of multiple independent windows (subsystem-health, thought-stream, boot-timeline, etc.). Each window is created in `overlay::create_panel_windows()` and can be shown/hidden independently.

### Panels Do Not Modify State
The debug overlay is read-only. All panels subscribe to state events and render them. No panel sends commands that modify agent behavior, memory, or SoulBox state. Panels can trigger actions like "reboot daemon-bus" but these are admin operations, not state modifications.

### Window Position Persistence
Each panel calls `useWindowDragSave()` to automatically save its position via the `save_window_position` Tauri command. Position is stored in Tauri's persistent store and restored on next launch.

---

## Multi-Window Rules

### Always-On-Top and Transparency Are Platform-Specific
Tauri window decorations, always-on-top, transparency, and click-through are configured in `ui/src-tauri/src/overlay.rs` and `tauri.conf.json`. These features rely on platform-specific APIs (WinAPI on Windows, Cocoa on macOS). Never assume cross-platform behavior without testing.

### Global Hotkey Registration
The overlay toggle hotkey (F12 default) is registered in `hotkey::register_overlay_hotkey()`. The key string is configurable in `ui/config/ui.toml`. Always use Tauri's `tauri-plugin-global-shortcut` — never raw platform APIs.

---

## Concurrency Rules

### Rust Backend: tokio Async
All async code in the Tauri backend uses tokio. gRPC calls, event streams, and file I/O are async. CPU-bound work goes in `spawn_blocking`.

### React Frontend: Single-Threaded
React runs on the main UI thread. All state updates via `setState` or `useTauriEvent` happen synchronously on this thread. Never spawn worker threads or use SharedArrayBuffer in React.

---

## Component Naming

- React components: `PascalCase` (e.g., `ThoughtStream`, `SubsystemHealth`)
- TypeScript files: `PascalCase.tsx` or `camelCase.ts`
- Rust Tauri commands: `snake_case` (e.g., `get_debug_snapshot`)
- Tauri event names: `kebab-case` (e.g., `subsystem-status-updated`)

---

## Logging

### Rust Backend
Use `tracing` exclusively. JSON-formatted structured logs. Log at `info` for lifecycle events, `error` for failures, `debug` for detailed diagnostics.

Required fields on error events:
- `error` — the error message
- `subsystem` or `component` — where the error occurred
- Additional context fields as appropriate

### React Frontend
Use `console.error` for errors, `console.warn` for warnings, `console.log` sparingly for debugging. Production builds should have minimal console output. Never log sensitive data (tokens, user messages) in production.

---

## Build and Tooling

- Rust backend: `cargo build -p sena-ui` (in `ui/src-tauri/`)
- Frontend: `pnpm install && pnpm build` (in `ui/`)
- Dev mode: `pnpm tauri dev` (from `ui/`)
- Linting: `pnpm lint` (ESLint), `cargo clippy -p sena-ui` (Rust)
- Formatting: `pnpm format` (Prettier), `cargo fmt` (Rust)

Never commit unformatted code. Always run linters before submitting PRs.
