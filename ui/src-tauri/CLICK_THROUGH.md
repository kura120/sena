# Click-Through Behavior — Sena UI Overlay

## Design Architecture

The Sena UI overlay consists of multiple independent Tauri windows:
- 1 invisible host window (1x1 transparent)
- 4 debug panel windows (subsystem-health, event-bus, chat, boot-timeline)
- 1 toast notification window

Each panel is its own Tauri `WebviewWindow`, not an overlay on top of a fullscreen transparent window.

## How Click-Through Works

### Between Windows
Clicks on desktop areas between/outside panels pass through **automatically** because:
- Each panel is a separate OS window
- The operating system naturally routes mouse events to whichever window is under the cursor
- When the cursor is not over a Tauri window, clicks go directly to whatever is beneath (desktop, other apps)

### Within Transparent Panel Areas
Each panel window has a transparent background (`.transparent(true)`) with CSS-defined content areas:
- Panel content uses `var(--bg-panel)` which is nearly opaque (`rgba(15, 15, 18, 0.94)`)
- These opaque areas capture mouse events normally
- The tiny transparent edges around `border-radius` corners pass clicks through naturally on Windows (WebView2 handles this)

### What This Means
No additional Rust code is required for click-through behavior. The combination of:
1. Window-level transparency (`.transparent(true)`)
2. CSS background definitions (opaque vs transparent areas)
3. WebView2's native handling of transparent window regions

...provides the correct click-through behavior by default.

### Alternative Approach (Not Needed)
If this behavior doesn't work as expected, Tauri v2 provides `window.set_ignore_cursor_events(true)`, but:
- That makes the ENTIRE window click-through, requiring coordinate-based toggling
- The current architecture avoids that complexity
- CSS boundaries already define clickable vs pass-through regions

## Testing Click-Through
1. Run the overlay (`Insert` key to toggle)
2. Click on panel content — should capture input
3. Click on transparent areas between panels — should pass through to desktop
4. Click on rounded corners — should pass through to desktop

If clicks on transparent areas DON'T pass through, file an issue with details about the WebView2 version and Windows build.
