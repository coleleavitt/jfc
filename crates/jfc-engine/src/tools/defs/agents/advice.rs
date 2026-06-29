use jfc_provider::ToolDef;

pub(super) fn advice_tool_defs() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "Advisor".into(),
            description: "Consult JFC's configured local/client-side advisor model for guidance. \
                Takes NO parameters — JFC snapshots your conversation and sends it through the \
                configured advisor provider/model as a normal local tool call. The advisor sees \
                the task, every tool call you've made, every result you've seen.\n\n\
                Call advisor BEFORE substantive work — before writing, before committing to an \
                interpretation, before building on an assumption. Also call when stuck, when \
                considering a change of approach, or when you believe the task is complete.\n\n\
                Give the advice serious weight. If you follow a step and it fails empirically, \
                adapt. Surface conflicts in another advisor call rather than silently switching."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        ToolDef {
            name: "StructuredOutput".into(),
            description: "Provide structured output matching the required JSON schema. \
                This tool is only available when the agent was spawned with a `schema` \
                parameter. Call it with a JSON object that validates against the schema. \
                On success, the result is returned to the parent agent as validated data."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "additionalProperties": true,
                "description": "JSON object matching the schema specified by the parent agent"
            }),
        },
        ToolDef {
            name: "Research".into(),
            description: "Run an agentic deep-research pass on a question: a model plans \
                and REFORMULATES each next sub-query from the evidence gathered so far \
                (read → decide → search → repeat), routing sub-queries to the best source \
                — general web, the local codebase (ripgrep), or specialised indexes \
                (arXiv, OpenAlex, Crossref, PubMed, Semantic Scholar, DOAJ, CORE, a named \
                university via `uni:`, Wikipedia, etc.) — then a model synthesises the \
                gathered evidence into one CITED answer. Mirrors claude.ai/Perplexity \
                deep research. Runs out-of-band — it does NOT consume the main \
                conversation's tools and returns a self-contained report.\n\n\
                Use when a question needs current/external or academic information across \
                multiple angles (background + latest developments + mechanism + \
                criticism), or wants both web and repo evidence — not a single lookup. \
                For a one-shot fact, use WebSearch instead. Set `export` to also write a \
                durable markdown+json artifact."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "question": {
                        "type": "string",
                        "description": "The research question to investigate."
                    },
                    "export": {
                        "type": "boolean",
                        "description": "When true, also write the report to a durable \
                            artifact file (markdown + json). Defaults to false."
                    }
                },
                "required": ["question"]
            }),
        },
        ToolDef {
            name: "Council".into(),
            description: "Convene a model council: fan a question out to several models in \
                parallel, then an arbiter model synthesises their independent answers into \
                one consolidated reply that surfaces agreement (higher confidence) and \
                disagreement (presents the options). Mirrors Perplexity's COUNCIL_RESEARCH \
                / Model Council flow. Runs out-of-band like the advisor.\n\n\
                Use for high-stakes or contested questions where cross-checking multiple \
                models is worth the extra cost — architecture decisions, ambiguous \
                trade-offs, correctness reviews. For a quick second opinion, use the \
                advisor instead."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "question": {
                        "type": "string",
                        "description": "The question to put to the council."
                    },
                    "models": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional explicit member model ids. When omitted, \
                            the council uses configured [council].members when present, \
                            otherwise the active model plus the local advisor model."
                    },
                    "intent": {
                        "type": "string",
                        "enum": ["diagnose", "audit", "plan", "evaluate", "explain", "create", "perspectives", "freeform"],
                        "description": "Optional council intent. Shapes member prompts and synthesis."
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["direct", "agentic"],
                        "description": "Optional mode override. direct is the fast tool-less council path; agentic runs read-only task-backed members that inspect the repo and return StructuredOutput."
                    },
                    "archive": {
                        "type": "boolean",
                        "description": "When true, write a durable .jfc/council artifact bundle."
                    },
                    "quorum": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Minimum successful member answers required before synthesis."
                    },
                    "retry_on_fail": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Additional attempts for each failed or timed-out council member."
                    },
                    "member_timeout_ms": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Per-member timeout in milliseconds. 0 disables the timeout."
                    }
                },
                "required": ["question"]
            }),
        },
        ToolDef {
            name: "AskModel".into(),
            description: "Ask a specific model a one-shot question mid-turn and get its reply \
                threaded back into this conversation. Unlike Council (parallel fan-out + \
                arbiter) this is a single direct call to ONE model — use it to pull a \
                different model into the current turn: e.g. ask `gpt-5.5` for its take while \
                you (Claude) keep driving, then react to its answer. The reply returns as \
                this tool's result, so you can challenge it, build on it, or ask a follow-up \
                with another AskModel call. Runs out-of-band (no tools, prose only) like the \
                advisor.\n\n\
                Use for cross-model second opinions, comparing how a different model family \
                reasons about the same prompt, or interleaving two models within one task. \
                The `model` is resolved against the configured providers (e.g. `gpt-5.5`, \
                `openai/gpt-5.5`, `claude-opus-4-7`)."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "model": {
                        "type": "string",
                        "description": "Model id to ask, resolved against configured providers \
                            (e.g. `gpt-5.5`, `openai/gpt-5.5`, `claude-opus-4-7`)."
                    },
                    "prompt": {
                        "type": "string",
                        "description": "The prompt / question to send to that model."
                    },
                    "system": {
                        "type": "string",
                        "description": "Optional system prompt to steer the asked model's role."
                    }
                },
                "required": ["model", "prompt"]
            }),
        },
    ]
}
