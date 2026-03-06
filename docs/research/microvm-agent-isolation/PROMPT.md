# Claude Code Prompt: MicroVM Agent Isolation System

> Use this prompt with Claude Code to implement the full system.
> Copy everything below the line into a new Claude Code session.

---

## System Prompt

You are building `rho-vmctl`, a Rust CLI + Nix flake system that creates ephemeral MicroVMs for running AI coding agents (Claude Code, OpenAI Codex, or any agent CLI) in hardware-isolated environments. The system uses microvm.nix with cloud-hypervisor as the default hypervisor.

### Project Structure

```
agent-vms/
├── flake.nix                    # Root flake — imports microvm.nix, defines VM templates
├── flake.lock
├── Cargo.toml                   # Rust workspace
├── crates/
│   └── rho-vmctl/
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs          # CLI entry point (clap)
│           ├── config.rs        # Project config (TOML)
│           ├── vm.rs            # VM lifecycle (create/start/stop/destroy)
│           ├── credentials.rs   # Credential injection
│           ├── attach.rs        # Session attachment (stdin/stdout forwarding)
│           ├── network.rs       # TAP interface + NAT setup
│           └── health.rs        # VM health checks
├── nix/
│   ├── base-vm.nix              # Base MicroVM NixOS module (shared across all VMs)
│   ├── profiles/
│   │   ├── claude-code.nix      # Claude Code agent profile
│   │   ├── codex.nix            # OpenAI Codex agent profile
│   │   └── generic.nix          # Generic agent profile (just shell + tools)
│   ├── toolchains/
│   │   ├── rust-nightly.nix     # Rust nightly toolchain module
│   │   ├── node.nix             # Node.js toolchain module
│   │   ├── python.nix           # Python toolchain module
│   │   └── go.nix               # Go toolchain module
│   └── secrets.nix              # sops-nix integration for host-side secrets
├── config/
│   └── example.toml             # Example project configuration
├── .sops.yaml                   # sops-nix key configuration
└── README.md
```

### Step 1: Nix Flake + Base VM Module

Create `flake.nix` with these inputs:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    microvm = {
      url = "github:microvm-nix/microvm.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    sops-nix = {
      url = "github:Mic92/sops-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };
}
```

Create `nix/base-vm.nix` — the base NixOS module shared by all agent VMs:

```nix
# Every VM gets this module. It sets up:
# - cloud-hypervisor as hypervisor (sub-second boot, Rust-based, virtiofs support)
# - 4 GiB RAM, 8 vCPUs (configurable via options)
# - virtiofs share of host /nix/store at /nix/.ro-store (read-only, avoids store duplication)
# - virtiofs share of project directory at /workspace
# - TAP network interface on 10.100.0.0/24 subnet
# - SSH server (for debugging, optional)
# - systemd credential loading for API keys
# - Base packages: git, curl, openssh, coreutils, findutils, ripgrep, fd
# - User "agent" with home /home/agent, /workspace as working directory
# - Ephemeral root (tmpfs) — nothing persists except /workspace volume
```

Requirements for `base-vm.nix`:
- Define NixOS options under `agentVm` namespace for: `projectPath`, `agentType`, `ram`, `vcpu`, `extraPackages`, `credentialNames`
- The `/workspace` virtiofs mount MUST be read-write
- The `/nix/store` virtiofs mount MUST be read-only
- Network: TAP interface, static IP from `10.100.0.{vmIndex}/24`, gateway `10.100.0.1`
- DNS: use host's resolv.conf or `1.1.1.1`
- Firewall: allow outbound HTTPS (443) only — block all other egress by default
- Create systemd service `agent-runner.service` that:
  1. Waits for network and credential files
  2. Sources credentials from `/run/credentials/agent-runner.service/`
  3. Starts the configured agent CLI in `/workspace`
  4. Restarts on failure with 5s backoff

### Step 2: Agent Profiles

Create Nix modules for each agent under `nix/profiles/`:

**`claude-code.nix`**:
```nix
# Imports base-vm.nix
# Adds: nodejs (for claude-code CLI), @anthropic-ai/claude-code npm package
# Sets agentVm.agentType = "claude-code"
# Credential: ANTHROPIC_API_KEY
# agent-runner.service ExecStart:
#   claude --dangerously-skip-permissions \
#     --print \
#     --output-format stream-json \
#     --max-turns 50
# Stdin/stdout forwarded via vsock or serial console
```

**`codex.nix`**:
```nix
# Imports base-vm.nix
# Adds: codex-cli (Rust binary from cargo or npm)
# Sets agentVm.agentType = "codex"
# Credential: OPENAI_API_KEY
# agent-runner.service ExecStart:
#   codex --approval-mode full-auto --quiet
```

**`generic.nix`**:
```nix
# Imports base-vm.nix
# No agent-specific packages — just the toolchain
# User SSHes in and runs whatever they want
# agentVm.agentType = "generic"
# No agent-runner.service (just sshd)
```

### Step 3: Toolchain Modules

Each toolchain module under `nix/toolchains/` adds project-specific build tools:

- `rust-nightly.nix`: fenix rust-nightly, cargo, rust-src, rust-analyzer, pkg-config
- `node.nix`: nodejs_22, pnpm, yarn
- `python.nix`: python312, pip, virtualenv, uv
- `go.nix`: go_1_23, gopls

These are composable — a project config can list multiple toolchains.

### Step 4: `rho-vmctl` Rust CLI

Build a Rust CLI using `clap` (derive API) with these subcommands:

```
rho-vmctl create <project-name>     # Generate VM config from project TOML
rho-vmctl start <project-name>      # Boot the MicroVM
rho-vmctl stop <project-name>       # Graceful shutdown
rho-vmctl destroy <project-name>    # Remove VM state and config
rho-vmctl attach <project-name>     # Attach to agent session (stdin/stdout)
rho-vmctl status [project-name]     # Show VM status (running/stopped/error)
rho-vmctl list                      # List all configured projects
rho-vmctl logs <project-name>       # Stream agent-runner.service journal
```

#### Project Configuration (TOML)

```toml
# ~/.config/rho-vmctl/projects/myproject.toml
[project]
name = "myproject"
path = "/home/user/code/myproject"   # Host path, mounted as /workspace

[vm]
ram = 4096          # MiB
vcpu = 8
hypervisor = "cloud-hypervisor"    # or "qemu", "firecracker"

[agent]
type = "claude-code"   # or "codex", "generic"
# Agent-specific config passed as env vars or CLI args
max_turns = 50
model = "claude-sonnet-4-20250514"

[toolchains]
include = ["rust-nightly", "node"]   # Composable toolchain modules

[network]
allow_egress = ["api.anthropic.com", "api.openai.com"]  # Allowlisted domains
ssh_port = 2222       # Host port forwarded to guest :22 (0 = disabled)

[credentials]
# Maps credential name → sops-nix secret path on host
ANTHROPIC_API_KEY = "anthropic/api-key"
# Or direct file path:
# ANTHROPIC_API_KEY = "/run/secrets/anthropic-api-key"
```

#### `vm.rs` — VM Lifecycle

```rust
// Key implementation details:

// create():
// 1. Read project TOML config
// 2. Generate a NixOS configuration by composing:
//    - base-vm.nix
//    - Selected agent profile (claude-code.nix, codex.nix, etc.)
//    - Selected toolchain modules
//    - Project-specific overrides (ram, vcpu, network rules)
// 3. Write the composed config to /var/lib/rho-vmctl/<project>/configuration.nix
// 4. Run `nix build` to produce the VM runner script
//    Command: nix build .#nixosConfigurations.<project>.config.microvm.runner --out-link /var/lib/rho-vmctl/<project>/runner
// 5. Set up TAP interface via `ip tuntap add` and bridge

// start():
// 1. Verify runner exists (else error: "run `create` first")
// 2. Resolve credentials: read sops-nix paths → copy to credential staging dir
// 3. Set up TAP interface + NAT rules (iptables/nftables)
// 4. Execute the runner script as a background process
// 5. Wait for VM to boot (poll vsock or SSH, timeout 10s)
// 6. Write PID file to /var/lib/rho-vmctl/<project>/vm.pid

// stop():
// 1. Send ACPI shutdown via cloud-hypervisor API socket
// 2. Wait up to 30s for graceful shutdown
// 3. SIGKILL if still running
// 4. Tear down TAP interface + NAT rules
// 5. Remove PID file

// destroy():
// 1. stop() if running
// 2. Remove /var/lib/rho-vmctl/<project>/ directory
// 3. Remove generated NixOS config
```

#### `credentials.rs` — Credential Injection

```rust
// Credential flow:
// 1. Parse project TOML [credentials] section
// 2. For each credential:
//    a. If value starts with "/" → treat as file path, read contents
//    b. Else → treat as sops-nix secret name, resolve via /run/secrets/<name>
// 3. Write credential values to staging dir: /run/rho-vmctl/<project>/credentials/<name>
//    - Permissions: 0600, owned by root
//    - These files are passed to microvm via `microvm.credentialFiles`
// 4. Inside the VM, systemd makes them available at:
//    /run/credentials/agent-runner.service/<name>
// 5. agent-runner.service sources them as environment variables

// IMPORTANT: Never write credentials to the Nix store.
// IMPORTANT: Clean up staging dir on VM stop/destroy.
```

#### `attach.rs` — Session Attachment

```rust
// Attach to a running agent's stdin/stdout.
// Two strategies (implement both, pick based on hypervisor):
//
// Strategy A: vsock (preferred for cloud-hypervisor)
// - The guest runs a vsock listener (socat or custom) on CID 3, port 5000
// - agent-runner.service pipes agent stdin/stdout through the vsock
// - rho-vmctl connects to vsock from host side
// - Bidirectional: host stdin → vsock → guest agent stdin
//                  guest agent stdout → vsock → host stdout
//
// Strategy B: SSH tunnel (fallback)
// - SSH into the VM, attach to agent tmux/screen session
// - Less elegant but works with all hypervisors
//
// The attach command should:
// 1. Put the host terminal in raw mode (crossterm::terminal::enable_raw_mode)
// 2. Forward stdin/stdout bidirectionally
// 3. Handle Ctrl-C as detach (not kill)
// 4. Restore terminal on detach
```

#### `network.rs` — Network Setup

```rust
// Per-VM network isolation:
//
// 1. Create TAP interface: `ip tuntap add dev tap-<project> mode tap`
// 2. Assign host-side IP: `ip addr add 10.100.0.1/24 dev tap-<project>`
// 3. Bring up: `ip link set tap-<project> up`
// 4. Enable IP forwarding: sysctl net.ipv4.ip_forward=1
// 5. NAT for outbound: `iptables -t nat -A POSTROUTING -s 10.100.0.0/24 -j MASQUERADE`
// 6. Egress filtering (per project config):
//    For each allowed domain in config.network.allow_egress:
//      Resolve domain → IP(s)
//      `iptables -A FORWARD -s 10.100.0.0/24 -d <ip> -p tcp --dport 443 -j ACCEPT`
//    Default: `iptables -A FORWARD -s 10.100.0.0/24 -j DROP`
//
// On stop/destroy: reverse all iptables rules, delete TAP interface
//
// VM index assignment: hash project name to get stable index in 2..254 range
// This gives each VM a deterministic IP: 10.100.0.<index>
```

### Step 5: Host NixOS Module

Create a NixOS module that can be imported into the host's configuration:

```nix
# nix/host-module.nix
# This module:
# 1. Imports microvm.nixosModules.host
# 2. Sets up the bridge network (10.100.0.0/24)
# 3. Enables IP forwarding
# 4. Installs rho-vmctl binary
# 5. Creates /var/lib/rho-vmctl/ state directory
# 6. Optionally enables sops-nix for secret management
# 7. Creates systemd services for auto-starting configured VMs
```

### Step 6: Integration Tests

Write integration tests (ignored by default, require KVM) that:

1. Build a minimal VM config with `generic.nix` profile
2. Boot it, verify SSH connectivity
3. Verify `/workspace` mount is writable
4. Verify `/nix/store` is shared and read-only
5. Verify credential files appear in `/run/credentials/`
6. Verify egress filtering (HTTPS to allowed domain works, other traffic blocked)
7. Stop VM, verify cleanup (TAP gone, PID file removed, credentials cleaned)

### Implementation Order

1. `flake.nix` + `nix/base-vm.nix` — get a VM booting with `nix run`
2. `nix/profiles/generic.nix` — SSH-able VM with toolchains
3. `rho-vmctl create` + `start` + `stop` — basic lifecycle
4. `credentials.rs` — credential injection pipeline
5. `nix/profiles/claude-code.nix` — Claude Code running in VM
6. `attach.rs` — session attachment via vsock
7. `network.rs` — egress filtering
8. `nix/profiles/codex.nix` — Codex support
9. Integration tests

### Engineering Constraints

- **Rust edition 2024** (nightly), same as the rho workspace
- Use `tokio` for async, `clap` derive for CLI, `serde` + `toml` for config, `anyhow` for errors
- Shell commands (ip, iptables, nix) via `tokio::process::Command` — parse output, handle failures
- No `unwrap()` in production paths
- TAP/iptables operations require root — check at startup, exit with clear error if not root
- All VM state under `/var/lib/rho-vmctl/<project>/` — no global state files
- Credential staging under `/run/rho-vmctl/<project>/credentials/` (tmpfs, cleaned on stop)
- Log to stderr with `tracing` crate, structured JSON in `--json` mode

### What Success Looks Like

```bash
# Configure a project
rho-vmctl create myproject --config ./myproject.toml

# Boot the VM (sub-second)
rho-vmctl start myproject
# → MicroVM 'myproject' started (cloud-hypervisor, 4 GiB, 8 vCPUs)
# → IP: 10.100.0.42, SSH: localhost:2222
# → Agent: claude-code (waiting for task)

# Attach to the agent session
rho-vmctl attach myproject
# → Connected to claude-code in myproject
# → (stdin/stdout forwarded, Ctrl-C to detach)

# Check status
rho-vmctl status
# → myproject    running    claude-code    4 GiB    8 vCPU    10.100.0.42    2m31s

# Tear down
rho-vmctl stop myproject
# → MicroVM 'myproject' stopped, credentials cleaned

# Full cleanup
rho-vmctl destroy myproject
# → MicroVM 'myproject' destroyed, config removed
```

The VM boots in under a second. The agent runs in hardware-isolated KVM, with its own kernel. Credentials never touch the Nix store. Network egress is restricted to API endpoints only. The project directory is shared via virtiofs — changes made by the agent are immediately visible on the host. When the VM stops, only the project files remain.
