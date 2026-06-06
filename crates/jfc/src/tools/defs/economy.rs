use jfc_provider::ToolDef;

pub fn economy_tool_defs() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "post_bounty".into(),
            description: "Register a coding-task bounty in the agent \
                economy market. By default this only registers — solvers \
                and validators DO NOT run until you also call \
                `run_bounty(bounty_id)`, OR pass `auto_dispatch: true` \
                here to register and run in one shot. Once dispatched, \
                multiple solver agents compete (real LLM sub-calls in \
                parallel git worktrees), validators adversarially challenge \
                each surviving solution (sealed sessions, no peer \
                pressure), and only solutions surviving validation are \
                ranked + paid. Budget is tracked as real LLM tokens; the \
                orchestrator's CFO layer gates spending so the cycle \
                can't exceed it. Use post+run when you want competitive, \
                cross-validated output instead of a single-shot edit. \
                Inspect state via `market_status` or /market."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "description": {
                        "type": "string",
                        "description": "What the task is. Concrete and self-contained — solvers won't see the surrounding conversation."
                    },
                    "budget": {
                        "type": "number",
                        "description": "Token budget for the entire bounty (all solvers + validators combined). Hard cap, enforced at runtime."
                    },
                    "acceptance_criteria": {
                        "type": "string",
                        "description": "Mechanistic pass/fail criteria — preferably commands like `cargo test --lib foo` that produce binary outcomes. Avoid soft criteria; agents will game them."
                    },
                    "max_solvers": {
                        "type": "number",
                        "description": "Optional cap on competing solvers (default from charter, typically 3). Range 1-5."
                    }
                },
                "required": ["description", "budget", "acceptance_criteria"]
            }),
        },
        ToolDef {
            name: "run_bounty".into(),
            description: "Drive an already-posted Open bounty through the \
                full Solve→Validate→Settle cycle. Pair this with \
                `post_bounty` (auto_dispatch=false) when you want to \
                register the bounty first and dispatch later — the post \
                step is cheap; this is the expensive step that actually \
                spawns solver + validator subagent LLM calls. Returns the \
                settlement (winner, total cost, payout count) when the \
                cycle completes. Errors fast if the bounty is not in \
                Open state or the provider isn't registered with the \
                tool layer."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "bounty_id": {
                        "type": "string",
                        "description": "The bounty ID returned by post_bounty (e.g. `bounty_a1a8…`)."
                    },
                    "max_solvers": {
                        "type": "number",
                        "description": "Optional override for the number of competing solvers (1-5, default 2)."
                    }
                },
                "required": ["bounty_id"]
            }),
        },
        ToolDef {
            name: "market_status".into(),
            description: "Read the agent economy's current state. Returns \
                bounty count, spend, composite health score (efficiency × \
                fairness × trust × budget; <0.3 = CRITICAL), and any agents \
                flagged for collusion / rubber-stamping / griefing. \
                Optionally pass `bounty_id` to get the specific bounty's \
                phase (Posting / Bidding / Executing / Validating / \
                Settling / Complete)."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "bounty_id": {
                        "type": "string",
                        "description": "Optional bounty ID to drill into. Omit for global market summary."
                    }
                }
            }),
        },
    ]
}
