# Prior Art: MicroVM Agent Isolation

## Existing Approaches

### 1. Michael Stapelberg's Coding Agent VMs (Feb 2026)

The closest existing implementation. NixOS host with microvm.nix, per-project VMs running Claude Code.

**Architecture**: NixOS host → `microvm.nixosModules.host` → per-project MicroVMs with cloud-hypervisor, 8 vCPUs, 4 GiB RAM, 8 GiB writable overlay. Network on `192.168.83.0/24` with TAP + NAT. Project source shared via virtiofs. Claude Code configured via home-manager.

**What's missing**: No CLI tooling for lifecycle management, no credential pipeline, no egress filtering, no multi-agent support. Everything is manually configured in the host's `flake.nix`.

**Reference**: https://michael.stapelberg.ch/posts/2026-02-01-coding-agent-microvm-nix/

### 2. Claude Code Built-in Sandboxing

Uses bubblewrap (Linux) / Seatbelt (macOS) for OS-level sandboxing. Restricts filesystem writes to CWD, blocks sensitive paths, proxies network through allowed domains.

**Limitations**:
- Shared kernel — container-level isolation, not hardware-level
- Known escape vectors: path tricks, ELF dynamic linker bypass, agents disabling their own sandbox
- Adds <15ms per command (negligible)
- The `@anthropic-ai/sandbox-runtime` npm package is open source

**Reference**: https://www.anthropic.com/engineering/claude-code-sandboxing

### 3. OpenAI Codex Cloud Mode

Runs in OpenAI-managed isolated containers. Internet disabled during task execution. Code provided via GitHub repos.

**Limitations**: Cloud-only, no self-hosted option, no customization of environment, limited to OpenAI models.

**Reference**: https://openai.com/index/introducing-codex/

### 4. Docker Sandboxes for Coding Agents (2026)

Docker's microVM-based isolation for macOS/Windows. Each agent gets a disposable VM. Unique feature: agents can build/run Docker containers while remaining isolated.

**Limitations**: Tied to Docker Desktop, not NixOS-native, limited Linux host support.

**Reference**: https://www.docker.com/blog/docker-sandboxes-run-claude-code-and-other-coding-agents-unsupervised-but-safely/

### 5. OpenSandbox (Alibaba, March 2026)

Open-source (Apache 2.0) execution platform supporting Claude Code, Gemini CLI, Codex.

**Reference**: https://en.cryptonomist.ch/2026/03/03/opensandbox-ai-sandbox-secure-execution/

## Hypervisor Comparison

| Hypervisor | Boot Time | virtiofs | vsock | Best For |
|---|---|---|---|---|
| **cloud-hypervisor** | ~500ms | Yes | Yes | Default choice — good balance of features + speed |
| **Firecracker** | ~125ms | No | Yes | Fastest boot, but no virtiofs (need block device for store) |
| **QEMU** | ~1-2s | Yes | Yes | Most featureful, but slowest boot and largest attack surface |
| **crosvm** | ~500ms | Yes (broken 9p) | Yes | Good alternative to cloud-hypervisor |
| **kvmtool** | ~200ms | No (9p only) | Yes | Lightweight but limited |

**Recommendation**: cloud-hypervisor as default. It supports virtiofs (critical for Nix store sharing), vsock (for session attachment), and boots fast enough. Firecracker is tempting for boot speed but lacks virtiofs, which means you'd need to copy the Nix store closure into a block device image — much slower overall.

## Credential Management Approaches

### Option A: `microvm.credentialFiles` (Recommended)

Host decrypts via sops-nix → writes to `/run/secrets/` → microvm passes file to guest via systemd `io.systemd.credential`. Guest reads from `/run/credentials/<service>/<name>`.

**Pros**: Clean separation, systemd-native, credentials never in Nix store, per-service scoping.
**Cons**: Requires sops-nix or equivalent on host.

### Option B: virtiofs Share of `/run/secrets`

Mount host's `/run/secrets` directory into the guest via virtiofs.

**Pros**: Simple, immediate access to all host secrets.
**Cons**: Guest sees ALL host secrets (not scoped), known remount issues on `nixos-rebuild switch`.

### Option C: Guest-Side sops-nix

Give the guest its own age key (via credentialFiles), run sops-nix inside the guest.

**Pros**: Most isolated — guest manages its own secrets independently.
**Cons**: More complex setup, age key still needs initial injection, slower (decrypt at boot).

### Option D: Environment Variables via NixOS Config

Set `environment.variables.ANTHROPIC_API_KEY = config.sops.secrets.api-key.path;` in the guest NixOS config.

**Pros**: Simplest.
**Cons**: Values end up in the Nix store (readable by anyone with store access). NOT RECOMMENDED for production.

**Decision**: Option A for production, Option D acceptable for local dev only.

## Network Isolation Strategies

### Egress Filtering

The core challenge: agents need HTTPS access to LLM API endpoints but nothing else.

**Approach 1: iptables per-IP**
Resolve allowed domains → IPs, create ACCEPT rules, default DROP. Simple but breaks on DNS changes.

**Approach 2: DNS-based filtering (nftables + dnsmasq)**
Run dnsmasq in the VM, log resolved IPs, dynamically update nftables sets. More robust but complex.

**Approach 3: HTTP proxy (squid/tinyproxy)**
Route all VM traffic through a host-side proxy with domain allowlist. Most flexible, handles SNI inspection.

**Recommendation**: Start with Approach 1 (iptables per-IP) for simplicity. Migrate to Approach 3 if DNS instability is a problem. The Rust bridge can refresh iptables rules periodically by re-resolving domains.

## Security Model

### Threat Model

| Threat | Mitigation |
|---|---|
| Agent escapes sandbox | VM provides hardware isolation (KVM). Even a kernel exploit inside the VM doesn't affect the host. |
| Agent exfiltrates data | Egress filtering limits outbound to API endpoints only. No arbitrary internet access. |
| Credential theft | Credentials scoped per-VM via systemd credentials. No cross-VM access. Cleaned on stop. |
| Prompt injection from untrusted files | Defense-in-depth: agent's own sandbox (bubblewrap) runs inside the VM. Even if bypassed, VM contains blast radius. |
| Supply chain attack via agent-installed packages | VM is ephemeral. Packages installed at runtime don't persist. Only /workspace survives reboot. |
| Agent modifies its own VM config | VM config is on the host, read-only from guest perspective. |

### Defense in Depth

Layer 1: Agent's built-in sandbox (bubblewrap/seatbelt) — catches most accidental escapes
Layer 2: MicroVM with separate kernel (KVM) — hardware-enforced isolation
Layer 3: Network egress filtering — limits data exfiltration surface
Layer 4: Credential scoping — each VM only sees its own API keys
Layer 5: Ephemeral root — no persistent state leaks between sessions

## Performance Expectations

| Metric | Value | Notes |
|---|---|---|
| VM boot | <1s | cloud-hypervisor with virtiofs |
| First `nix build` in VM | 0s | Store shared from host, no build needed |
| File I/O (virtiofs) | ~80-90% native | Negligible overhead for code editing workloads |
| Network latency | +0.1ms | TAP interface overhead |
| Memory overhead | ~5 MiB per VM | Hypervisor process itself; VM RAM is the main cost |
| Agent startup | 2-5s | Node.js/Rust CLI cold start inside VM |

## Open Questions

1. **macOS support**: VFKit is the only macOS hypervisor in microvm.nix but lacks TAP networking and virtiofs. Is macOS a target? If so, might need a different approach (Lima, OrbStack, or Docker-based).

2. **Multi-VM networking**: Should VMs be able to talk to each other? (e.g., agent in VM-A making API calls to a service in VM-B). Current design isolates each VM independently.

3. **Persistent VM mode**: Current design is fully ephemeral. Should there be an option for persistent VMs that survive host reboots? (Use case: long-running agent sessions.)

4. **GPU passthrough**: Some agents may benefit from GPU access (e.g., local model inference). QEMU and cloud-hypervisor support VFIO GPU passthrough, but it's complex to configure.

5. **Resource limits**: Should there be cgroup-style resource limits on VM processes? cloud-hypervisor already constrains CPU and memory, but I/O bandwidth and network rate limiting may be useful.
