# Sena

A local-first AI assistant built to actually know you: your habits, your context, your work. Nothing leaves your machine.

No cloud. No subscriptions. No data sent anywhere.

> [!NOTE]
> Active development. Not ready for general use.

## What it is

Sena is a personal AI companion designed to run entirely on your hardware. It maintains persistent memory across sessions, routes tasks to specialized agents, and adapts to how you work over time.

The goal isn't a smarter chatbot. It's a system that builds a real model of you.

## How it's built

Rust and Python, running locally via [llama-cpp-rs](https://github.com/utilityai/llama-cpp-rs). Memory is powered by [ech0](https://github.com/kura120/ech0), a local knowledge graph crate built alongside this project.

The architecture is modular, subsystems communicate over an internal message bus, each with a defined scope and no shared state.

## Status

| Subsystem | Language | Status |
|---|---|---|
| daemon-bus | Rust | ✅ Scaffolded |
| model-probe | Rust | 🔨 In progress |
| memory-engine | Rust | 🔨 In progress |
| ech0 | Rust | 🔨 In progress |
| CTP | Python | 📋 Planned |
| prompt-composer | Python | 📋 Planned |
| Tacet | Python | 📋 Planned |
| SoulBox | Python | 📋 Planned |
| lora-manager | Python | 📋 Planned |
| codebase-context | Python + Rust | 📋 Planned |

## Motivation

Most AI assistants are built to keep you dependent on a service. Sena is built to run on your machine, know your context, and stay yours.