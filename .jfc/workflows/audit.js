export const meta = {
  name: "audit",
  description: "Whole-codebase structural audit using jfc-graph. Identifies untested code, high-complexity functions, unreachable code, and taint flows from public entrypoints to sinks.",
  phases: [
    { title: "Map public entrypoints" },
    { title: "Analyze complexity hotspots" },
    { title: "Detect untested code paths" },
    { title: "Check reachability" },
    { title: "Synthesize findings" }
  ]
};

// Phase 1: Map public entrypoints and understand the attack surface
const entrypoints = await agent({
  prompt: `You are auditing this codebase for bugs and structural issues.

Step 1: Map the public API surface.
- Use \`graph_query\` with query: \`entrypoints kind=PublicApi\`
- Also run \`entrypoints kind=Main\` to find main entry points
- List all public-facing functions that accept external input

Report: a numbered list of all public entrypoints with their file:line locations and what parameters they accept.`,
  model: "anthropic/claude-sonnet-4-20250514"
});

// Phase 2: Find complexity hotspots
const complexity = await agent({
  prompt: `You are auditing this codebase for structural issues.

Step 2: Find complexity hotspots — functions that are too complex and likely to harbor bugs.
- Use \`code_index\` to get an overview of all functions
- Use \`graph_node\` with includeCode=true on the largest/most complex-looking functions
- Look for functions with:
  - Deep nesting (>4 levels)
  - Many branches (high cyclomatic complexity)
  - Functions longer than 80 lines
  - Functions doing too many things (mixed concerns)

Previous findings (entrypoints):
${entrypoints}

Report: a ranked list of complexity hotspots with file:line, function name, and WHY it's a concern.`,
  model: "anthropic/claude-sonnet-4-20250514"
});

// Phase 3: Find untested code
const untested = await agent({
  prompt: `You are auditing this codebase for test coverage gaps.

Step 3: Find untested code paths.
- Use \`run_coverage\` to annotate the graph with coverage data (if tests exist)
- Use \`graph_query\` with: \`entrypoints kind=PublicApi | untested\` to find untested public APIs
- Look for error paths that have no test coverage
- Check if critical functions identified in earlier phases have tests

Previous findings:
Entrypoints: ${entrypoints}
Complexity hotspots: ${complexity}

Report: list of untested functions/paths ranked by risk (public + complex + untested = highest risk).`,
  model: "anthropic/claude-sonnet-4-20250514"
});

// Phase 4: Check reachability — find dead code
const reachability = await agent({
  prompt: `You are auditing this codebase for dead/unreachable code.

Step 4: Find unreachable code.
- Use \`graph_query\` to find functions with no callers: look for nodes that aren't called by anything AND aren't entrypoints
- Use \`graph_callers\` on suspicious functions to verify they have zero callers
- Check for unused struct/enum definitions
- Look for code that's been superseded but not removed

Previous findings:
Entrypoints: ${entrypoints}

Report: list of likely dead code with file:line locations and confidence level.`,
  model: "anthropic/claude-sonnet-4-20250514"
});

// Phase 5: Synthesize all findings into a prioritized report
const report = await agent({
  prompt: `You are writing the final audit report. Synthesize all findings into a prioritized, actionable report.

## All Findings

### Entrypoints (attack surface):
${entrypoints}

### Complexity hotspots:
${complexity}

### Untested code:
${untested}

### Dead/unreachable code:
${reachability}

## Your Task

Write a structured audit report with:
1. **Critical** — bugs/issues that could cause failures in production (untested public APIs with high complexity)
2. **High** — structural risks that make bugs likely (complex functions without tests)
3. **Medium** — code quality issues (dead code, unnecessary complexity)
4. **Low** — minor cleanup opportunities

For each finding, include:
- File:line location
- What the issue is
- Why it matters
- Suggested fix (one sentence)

End with a summary: total findings by severity, and the top 3 things to fix first.`,
  model: "anthropic/claude-sonnet-4-20250514"
});
