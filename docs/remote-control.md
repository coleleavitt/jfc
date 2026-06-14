# Remote Control

Run a jfc session on one machine and drive it from another device — a
laptop terminal, a phone browser, or any WebSocket client.

## Quick Start

### On the host machine (where jfc is running)

```bash
# Inside a jfc session:
/remote-control

# jfc prints:
#   Remote control enabled
#   jfc rc connect ws://127.0.0.1:4242 --token <TOKEN>
```

### On the client device

```bash
jfc rc connect ws://127.0.0.1:4242 --token <TOKEN>
```

Type a line and press Enter to send a prompt. `/interrupt` sends an
Esc (cancel). `y` / `n` respond to tool permission and plan-approval
prompts that the host forwards.

## Reaching the Host from Another Machine

The WebSocket server binds to `127.0.0.1:4242` by default — it is
**never** exposed directly to the network. You use an encrypted tunnel
to reach it remotely:

### Option 1: Tailscale (recommended if you already use it)

On the host:

```bash
tailscale serve https+insecure://localhost:4242
```

On the client:

```bash
jfc rc connect ws://<tailscale-ip>:4242 --token <TOKEN>
```

Zero extra config. Traffic is WireGuard-encrypted end-to-end.

### Option 2: SSH tunnel (universal, no third party)

On the client:

```bash
ssh -L 4242:localhost:4242 user@host
# then, in another terminal on the client:
jfc rc connect ws://localhost:4242 --token <TOKEN>
```

The SSH session must stay open for the tunnel to work.

### Option 3: Cloudflared (public URL, works from a phone)

On the host:

```bash
cloudflared tunnel --url http://localhost:4242
```

Cloudflared prints a `https://<random>.trycloudflare.com` URL. On the
client:

```bash
jfc rc connect wss://<random>.trycloudflare.com --token <TOKEN>
```

No Cloudflare account needed for quick tunnels. For persistent setups,
create a named tunnel.

## Security Model

| Layer | How it works |
| --- | --- |
| **Bearer token** | The WS client sends the pairing token on the HTTP upgrade handshake (`Sec-WebSocket-Protocol: bearer.<token>`). The server rejects connections with wrong tokens. |
| **HMAC-SHA256 per frame** | Every `RemoteFrame` carries an HMAC over `"{version}.{seq}.{ts_ms}.{payload_json}"`. A relay or MITM cannot forge events. |
| **Monotonic sequence numbers** | Each direction tracks the last accepted `seq` and rejects replays or out-of-order frames. |
| **Localhost binding** | The WS server never binds to `0.0.0.0`. All remote access goes through an encrypted tunnel. |
| **Tunnel encryption** | Tailscale = WireGuard, SSH = SSH ciphers, cloudflared = TLS. The WS payload is plaintext *inside* the tunnel. |

The pairing token is generated per-session (32 cryptographically random
bytes, base64-encoded). It is printed to the host terminal and must be
copied out-of-band.

## Wire Protocol

The protocol is defined in the `jfc-remote` crate (`protocol.rs`).

**Outbound (host → client):**

| Variant | Payload |
| --- | --- |
| `assistant_delta` | `text`, `reasoning` |
| `tool_use` | `id`, `name`, `input_preview` |
| `tool_result` | `id`, `output_preview`, `is_error` |
| `session_status` | `status` (`running`/`idle`/`waiting_approval`/`terminated`/`error`), `message` |
| `permission_request` | `tool_use_id`, `tool_name`, `summary`, `diff` |
| `plan_approval_request` | `plan` |
| `toast` | `kind`, `text` |
| `heartbeat` | (empty) |

**Inbound (client → host):**

| Variant | Payload |
| --- | --- |
| `user_prompt` | `text` |
| `interrupt` | (empty — equivalent to pressing Esc) |
| `approval_response` | `tool_use_id`, `approved` |
| `plan_approval_response` | `approve`, `feedback` |
| `ping` | (empty) |

Each frame: `RemoteFrame { version, seq, ts_ms, payload, hmac }`.

## Slash Commands

| Command | Effect |
| --- | --- |
| `/remote-control` (alias `/rc`) | Enable RC on the current session |
| `/rc off` | Disable RC and disconnect clients |
| `/rc status` | Show RC state + connected client count |

## CLI Subcommands

| Command | Description |
| --- | --- |
| `jfc rc connect <url> --token <tok>` | Connect as a client |
| `jfc rc status <url> --token <tok>` | Probe a server's reachability |

## Approving tools remotely

When the host hits a tool that needs approval, every connected client
receives a `PermissionRequest` with a diff preview (the same +/- lines
the host's approval modal shows). Reply `y` or `n` on the next line:

```
🔒 permission: Edit — src/main.rs
  --- src/main.rs
  +++ src/main.rs
  - let x = 1;
  + let x = 2;
   → y to approve, n to reject (then Enter)
y
```

Plan-mode approvals (`📋 plan approval requested`) work the same way;
a non-`y` reply is sent as rejection feedback.

## Limitations

- **Line-oriented client**: `jfc rc connect` is a simple stdin/stdout
  bridge, not a full ratatui mirror of the host TUI. Functional, not
  pixel-perfect.
- **No daemon spawn mode**: `/remote-control` mirrors a *running*
  session; it can't yet *launch* new sessions from a client (Claude
  Code's "spawn mode"). Daemon-hosted RC is a future enhancement.

Multiple clients are supported — every connected device receives the
same mirrored event stream (fan-out via `tokio::broadcast`), and any
client can send prompts / approvals.
