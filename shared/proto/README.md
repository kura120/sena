This directory is the canonical home of all gRPC proto definitions.

## Current State

All subsystem `build.rs` files reference `shared/proto/sena.daemonbus.v1.proto`.
The legacy copy in `daemon-bus/proto/` is retained for reference but is no longer
the source of truth.

## Rules

- **All proto definitions live here** — never create proto files elsewhere.
- Proto changes require updating implementations in all affected subsystems.
- Current proto package: `sena.daemonbus.v1`
