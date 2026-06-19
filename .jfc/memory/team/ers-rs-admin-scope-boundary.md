---
type: project
scope: team
created: 2026-06-15
---
ers-rs `ers-internal-ui` admin→rep-scope is a DELIBERATE security boundary, not a bug. The `ers-internal-ui` OAuth client is a public/PKCE client (tokens in browser localStorage) allow-listed for `read:*` scopes ONLY (`clickhouse/init/016_oauth_client_boundaries.sql`); the backend grants operator-admin only when the token carries `admin:*` (`crates/ers-api/src/auth_context.rs` `from_jwt_claims`: `is_admin = role.is_operator_admin() && has_scope(scope, SCOPE_ADMIN_ALL)`), verified by tests `oauth_admin_role_without_admin_scope_is_not_admin` / `_with_admin_scope_is_admin` (14/14 auth_context tests pass). **Why:** a public browser client must not wield admin power even when an admin user signs in (XSS/stolen-token defense against production). **How to apply:** do NOT add `admin:*` to that client or treat role==admin as admin without the scope — that dismantles the boundary and needs a prod write + explicit sign-off. Open decision doc: `.claude/decisions/admin-access-in-internal-ui.md`. Recommended path = separate admin client with a server-side confidential flow.
