#!/usr/bin/env bash
# Sena — launch daemon-bus and UI for development
# Usage: ./scripts/dev.sh [--daemon-only | --ui-only]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

cleanup() {
    echo ""
    echo "[sena] shutting down..."
    if [[ -n "${DAEMON_PID:-}" ]]; then
        kill "$DAEMON_PID" 2>/dev/null || true
        wait "$DAEMON_PID" 2>/dev/null || true
    fi
    if [[ -n "${UI_PID:-}" ]]; then
        kill "$UI_PID" 2>/dev/null || true
        wait "$UI_PID" 2>/dev/null || true
    fi
    echo "[sena] stopped."
}
trap cleanup EXIT INT TERM

MODE="${1:-all}"

start_daemon() {
    echo "[sena] starting daemon-bus..."
    cd "$ROOT_DIR"
    cargo run --bin daemon-bus &
    DAEMON_PID=$!
    echo "[sena] daemon-bus PID: $DAEMON_PID"
}

start_ui() {
    echo "[sena] starting ui (tauri dev)..."
    cd "$ROOT_DIR/ui"
    pnpm tauri dev &
    UI_PID=$!
    echo "[sena] ui PID: $UI_PID"
}

case "$MODE" in
    --daemon-only)
        start_daemon
        wait "$DAEMON_PID"
        ;;
    --ui-only)
        start_ui
        wait "$UI_PID"
        ;;
    all|*)
        start_daemon
        # Give the daemon a moment to bind its gRPC port
        sleep 2
        start_ui
        wait "$DAEMON_PID" "$UI_PID"
        ;;
esac
