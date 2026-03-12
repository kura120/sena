<p align="center">
  <a href="https://github.com/kura120/sena">
    <img src="assets/sena-github-banner.png" width="800px" alt="sena">
  </a>
</p>

<p align="center">
  <a href="#introduction">Introduction</a> ·
  <a href="#techstack">Tech Stack</a> ·
  <a href="#status">Status</a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Status-In%20Development-red">
  <img src="https://img.shields.io/badge/License-MIT-green.svg?logo=data:image/svg%2bxml;base64,PHN2ZyB3aWR0aD0iMTk3cHgiIGhlaWdodD0iMTk3cHgiIHhtbG5zPSJodHRwOi8vd3d3LnczLm9yZy8yMDAwL3N2ZyIgdmVyc2lvbj0iMS4xIj4KICAgIDxjaXJjbGUgY3g9Ijk4IiBjeT0iOTgiIHI9IjkwIiBmaWxsPSJ3aGl0ZSIvPgoJPGNpcmNsZSBjeD0iOTgiIGN5PSI5OCIgcj0iNzgiIGZpbGw9IiM1NTU1NTUiLz4KCTxjaXJjbGUgY3g9Ijk4IiBjeT0iOTgiIHI9IjU1IiBmaWxsPSJ3aGl0ZSIvPgoJPGNpcmNsZSBjeD0iOTgiIGN5PSI5OCIgcj0iMzAiIGZpbGw9IiM1NTU1NTUiLz4KCTxyZWN0IHg9IjExNSIgeT0iODUiIHdpZHRoPSI0NSIgaGVpZ2h0PSIyNSIgZmlsbD0iIzU1NTU1NSIvPgo8L3N2Zz4=" alt="MIT License">
</p>


A local-first AI assistant built to actually know you: your habits, your context, your work. Nothing leaves your machine.

No cloud. No subscriptions. No data sent anywhere.

> [!NOTE]
> Active development. Not ready for general use.

## Introduction <a name="introduction"></a>

Sena is a personal AI companion designed to run entirely on your hardware. It maintains persistent memory across sessions, routes tasks to specialized agents, and adapts to how you work over time.

The goal isn't a smarter chatbot. It's a system that builds a real model of you.

## Tech Stack <a name="techstack"></a>

Rust and Python, running locally via [llama-cpp-rs](https://github.com/utilityai/llama-cpp-rs). Memory is powered by [ech0](https://github.com/kura120/ech0), a local knowledge graph crate built alongside this project.

The architecture is modular, subsystems communicate over an internal message bus, each with a defined scope and no shared state.

## Status <a name="status"></a>

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