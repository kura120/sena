---
applyTo: "ui/**"
---

# ui — Copilot Instructions

The UI subsystem is built with Freya — a Rust-native GUI framework using Skia as its rendering backend (the same engine behind Chrome, Flutter, and Figma). Freya uses a React-like component model with hooks. The UI's job is to render state and deliver events. It makes no decisions, holds no business logic, and never touches memory or agents directly.

These rules are traps specific to this subsystem. Global rules in `.github/copilot-instructions.md` also apply in full.

---

## Ownership Boundaries

UI owns:
- All visual components and layout
- Receiving state updates from daemon-bus via gRPC and rendering them
- Sending user input events to daemon-bus via gRPC
- Animation and visual transitions

UI does not own:
- Any business logic — the UI renders state, it never makes decisions
- Any memory reads or writes
- Any agent calls
- Any SoulBox reads directly — SoulBox state arrives via daemon-bus events
- Any model calls

If you find yourself writing logic that determines what Sena should do inside a UI component, stop. That logic belongs in an agent or CTP.

---

## Freya-Specific Traps

### State Lives in the Backend — UI Only Renders
The UI receives state snapshots via gRPC events. It never owns the source of truth for any data.

```rust
// bad — UI owns state
#[component]
fn ChatWindow() -> Element {
    let messages = use_signal(|| fetch_messages_from_memory()); // wrong
    ...
}

// good — state arrives via event
#[component]
fn ChatWindow() -> Element {
    let messages = use_context::<AppState>().messages; // rendered from received state
    ...
}
```

### No Blocking Calls in Components
Never make synchronous blocking calls inside a Freya component or hook. All backend communication is async via gRPC. Use Freya's async hooks for any I/O.

```rust
// bad — blocks render thread
#[component]
fn StatusBar() -> Element {
    let status = get_status_sync(); // blocks
    ...
}

// good — async hook
#[component]
fn StatusBar() -> Element {
    let status = use_future(|| async { get_status().await });
    ...
}
```

### Components Are Pure When Possible
A component given the same props must produce the same output. Never read from global mutable state inside a component. Pass all data through props or context.

### Animations Use Freya's Built-In Animation API
Never implement animations manually with timers or sleep loops. Always use Freya's built-in animation primitives with explicit easing and duration.

```rust
// bad — manual timer animation
use_effect(move || {
    spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(16)).await;
            opacity.set(*opacity.read() + 0.1);
        }
    });
});

// good — Freya animation API
let animation = use_animation(|conf| {
    conf.on_creation(AnimNum::new(0.0, 1.0)
        .duration(Duration::from_millis(300))
        .easing(Easing::EaseOut))
});
```

### No Hardcoded Strings in Components
All user-facing text is passed as props or comes from a localization context. Never write literal UI text in a component.

```rust
// bad
rsx! { p { "Hello, I'm Sena" } }

// good — text from context or props
rsx! { p { "{greeting_text}" } }
```

---

## Debug UI Traps

### Debug UI Is a Separate Component Tree
The debug panel (CTP state, memory ops, agent states, hierarchy tiers, telemetry) is a completely separate component tree from the main chat UI. Never mix debug components into production UI components.

### Debug UI Never Modifies State
The debug panel is read-only. It subscribes to state events and renders them. It never sends commands that modify agent behavior, memory, or SoulBox state.

### Debug Data Is Streamed — Never Polled
Debug panel data (CTP thought queue, memory ops, telemetry) arrives via gRPC server streaming. Never poll for debug data on a timer.

```rust
// bad — polling
use_effect(move || {
    spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            fetch_debug_state().await;
        }
    });
});

// good — streaming subscription
use_effect(move || {
    spawn(async move {
        let mut stream = daemon_bus.subscribe_debug_stream().await;
        while let Some(event) = stream.next().await {
            debug_state.set(event);
        }
    });
});
```

---

## Concurrency Rules

Freya renders on a single foreground thread. All rendering and state updates happen on this thread. Background work (gRPC calls, event streams) runs on spawned async tasks and sends results back to the foreground thread via signals.

Never update a Freya signal from a background thread directly — always pass data back through the async task that owns the signal.

---

## Component Naming

- All components use `PascalCase`
- All props structs use `PascalCase` with `Props` suffix: `ChatWindowProps`
- All signals use `snake_case`
- File names match component names: `ChatWindow` lives in `chat_window.rs`

Never mix naming conventions — Freya's macro system is sensitive to naming and silent failures are possible.

---

## Logging

Use the `tracing` crate exclusively. Log at `debug` level for render events only during development. In production, UI logging is minimal — only log errors and significant lifecycle events.

Required fields on UI error events:
- `component` — which component produced the error
- `event_type` — render_failed, stream_disconnected, grpc_error
