# MicroVM + Nix: Isolated Environments for Agentic Coding Tools

## Problem Statement

AI coding agents (Claude Code, OpenAI Codex, etc.) need isolated execution environments that provide:

- **Hardware-level isolation** (not just container namespaces — separate kernel per VM)
- **Declarative, reproducible toolchains** per project (Nix guarantees identical builds)
- **Credential injection** without leaking secrets into the Nix store or VM image
- **Ephemeral by default** — tmpfs root, only project data persists
- **Sub-second boot** — cloud-hypervisor or Firecracker, not full QEMU
- **Host Nix store sharing** — virtiofs avoids duplicating `/nix/store` in every VM

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│  NixOS Host                                             │
│                                                         │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐     │
│  │  MicroVM A  │  │  MicroVM B  │  │  MicroVM C  │     │
│  │  (project1) │  │  (project2) │  │  (project3) │     │
│  │             │  │             │  │             │     │
│  │  claude-code│  │  codex-cli  │  │  claude-code│     │
│  │  rust, nix  │  │  node, py   │  │  go, nix    │     │
│  │             │  │             │  │             │     │
│  │ virtiofs:   │  │ virtiofs:   │  │ virtiofs:   │     │
│  │  /nix/store │  │  /nix/store │  │  /nix/store │     │
│  │  /project   │  │  /project   │  │  /project   │     │
│  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘     │
│         │                │                │             │
│  ┌──────┴────────────────┴────────────────┴──────┐      │
│  │              rho-vmctl (Rust bridge)           │      │
│  │  - VM lifecycle (create/start/stop/destroy)   │      │
│  │  - Credential injection via credentialFiles   │      │
│  │  - Project ↔ VM volume mapping                │      │
│  │  - Agent session forwarding (stdin/stdout)    │      │
│  │  - Health checks & resource monitoring        │      │
│  └───────────────────────────────────────────────┘      │
│                                                         │
│  /nix/store (shared read-only via virtiofs)              │
│  /var/lib/microvms/ (VM state)                          │
│  /run/secrets/ (sops-nix decrypted credentials)         │
└─────────────────────────────────────────────────────────┘
```

## Key Components

### 1. Nix Flake — VM Definitions

Each project gets a MicroVM NixOS configuration declared in a flake. The VM includes only the tools needed for that project.

### 2. `rho-vmctl` — Rust Bridge CLI

A Rust binary that manages VM lifecycle, credential injection, agent session attachment, and project-to-VM mapping. This is the glue between the host and microvm.nix.

### 3. Credential Pipeline

API keys flow: `sops-nix` on host → `/run/secrets/` → `microvm.credentialFiles` → guest systemd credentials → agent environment. Never touches the Nix store.

### 4. Agent Runner

Inside the VM, a systemd service starts the coding agent (claude-code, codex, etc.) with the project mounted at `/workspace`, credentials in environment, and network policy applied.

## Files in This Research

| File | Description |
|------|-------------|
| `README.md` | This overview |
| `PROMPT.md` | Claude Code prompt for implementing the full system |
| `prior-art.md` | Research on existing approaches, references, and tradeoffs |

## References

- [microvm.nix](https://github.com/microvm-nix/microvm.nix) — Nix flake for declarative MicroVMs
- [Coding Agent VMs on NixOS](https://michael.stapelberg.ch/posts/2026-02-01-coding-agent-microvm-nix/) — Michael Stapelberg's guide
- [Claude Code Sandboxing](https://www.anthropic.com/engineering/claude-code-sandboxing) — Anthropic's approach
- [Docker Sandboxes for Coding Agents](https://www.docker.com/blog/docker-sandboxes-run-claude-code-and-other-coding-agents-unsupervised-but-safely/)
- [sops-nix](https://github.com/Mic92/sops-nix) — Secret management for NixOS
