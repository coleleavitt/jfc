# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in jfc, **please do not open a public issue.**

Instead, report it privately by either:

- Using GitHub's [private vulnerability reporting](https://github.com/coleleavitt/jfc/security/advisories/new) (preferred), or
- Emailing **cole@unwrap.rs**

Include the following in your report:

- Description of the vulnerability
- Steps to reproduce
- Affected crates/modules (if known)
- Potential impact assessment

## Response Timeline

- **Acknowledgement**: within 48 hours
- **Initial assessment**: within 1 week
- **Fix timeline**: depends on severity, but we aim for patches within 2 weeks for critical issues

## Scope

The following areas are in scope for security reports:

| Area | Crate | Notes |
| --- | --- | --- |
| **Sandbox escape** | `jfc-ui` (sandbox module) | bubblewrap + Landlock bypasses |
| **Shell injection** | `jfc-ui` (shell_safety) | Bash tool command parsing |
| **OAuth credential leaks** | `jfc-auth`, `jfc-providers` | Token storage, credential vault |
| **Arbitrary code execution** | `jfc-economy` | Solver/validator agent isolation |
| **Path traversal** | `jfc-ui` (tools) | Read/Write/Edit tool path handling |
| **Taint bypass** | `jfc-graph`, `jfc-audit` | False negatives in taint analysis |
| **MCP injection** | `jfc-mcp` | Tool dispatch, protocol parsing |

## Out of Scope

- Vulnerabilities in upstream dependencies (report to the upstream project)
- Issues requiring physical access to the machine
- Denial of service via resource exhaustion (the tool runs locally)

## Disclosure Policy

We follow coordinated disclosure. Once a fix is available, we will:

1. Release the patch
2. Credit the reporter (unless anonymity is requested)
3. Add a note to the CHANGELOG

## Security Architecture

jfc includes built-in security tooling:

- **`jfc-audit`** crate: automated taint analysis, reachability checking, and attack surface enumeration
- **Shell safety parser**: blocks dangerous shell patterns (command injection, sed exec, etc.)
- **Sandbox**: bubblewrap (bwrap) + Landlock LSM for Bash tool execution
- **Permission modes**: Plan, Default, AcceptEdits, Auto, Bypass with per-tool approval
- **Bridge attestation**: request body integrity verification for OAuth flows
