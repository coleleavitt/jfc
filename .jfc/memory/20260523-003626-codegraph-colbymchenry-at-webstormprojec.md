---
type: context
scope: team
created: 2026-05-23T00:36:26.363230885+00:00
---
codegraph (colbymchenry) at ~/WebstormProjects/forks/codegraph is a SQLite tree-sitter knowledge graph + MCP server published as `@colbymchenry/codegraph`. Architecture: ExtractionOrchestrator → DB (nodes/edges/files) → ReferenceResolver → GraphTraverser → ContextBuilder. The MCP tools (codegraph_search/context/callers/callees/impact/node/explore/files/status) are the same tools jfc's MCP server exposes (we have parity).

Why: jfc-graph is a Rust re-implementation of this same shape, with a richer DSL (set algebra, taint, path patterns, preconditions, untested operator) — we already have MORE than codegraph.

How to apply: When researching graph-based features for jfc, codegraph is a comparison point, not a dependency. Their `src/mcp/server-instructions.ts` is the canonical "how should an agent use a code graph" guide — we already mirror this. Their installer + framework-route detection (Django, FastAPI, Express, etc.) is the most interesting gap to consider porting if/when jfc-graph wants framework-aware route nodes.
