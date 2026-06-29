use jfc_provider::ToolDef;

pub(super) fn skill_invocation_tool_defs() -> Vec<ToolDef> {
    vec![ToolDef {
        name: "Skill".into(),
        description: "Invoke a user-invocable registered skill by name. The skill body is rendered with runtime placeholders, attached package files are surfaced as readable paths, and `context: fork` skills run through the subagent path when invoked as slash commands. Pass `args` as additional context.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The registered skill name (matches the `name` frontmatter or filename stem under `.claude/skills/`)"
                },
                "skill": {
                    "type": "string",
                    "description": "Alias for `name`, accepted for Claude Code compatibility"
                },
                "args": {
                    "type": "string",
                    "description": "Optional additional context appended to the skill body"
                }
            },
            "required": ["name"]
        }),
    }]
}

pub(super) fn skill_authoring_tool_defs() -> Vec<ToolDef> {
    vec![ToolDef {
        name: "SkillCreate".into(),
        description: "Author a NEW reusable skill from a procedure you just performed, so \
            future sessions can invoke it by name via the Skill tool. This is the \
            write-half of the skill-from-experience loop: when you notice you've \
            repeated a multi-step workflow that worked, distil it into a skill. \
            Writes `.claude/skills/<name>/SKILL.md` with `created-by: agent` \
            provenance. Refuses to overwrite an existing skill (pick a fresh name). \
            The `name` must be a kebab-case slug.\n\n\
            Prefer this over re-deriving the same procedure each time. The body \
            should be concrete, step-by-step instructions (Markdown), not prose about \
            what a skill is."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Kebab-case skill name / slug (e.g. `rust-crate-bump`)."
                },
                "description": {
                    "type": "string",
                    "description": "One-line description of when to use this skill."
                },
                "body": {
                    "type": "string",
                    "description": "The skill body: concrete step-by-step Markdown instructions."
                }
            },
            "required": ["name", "description", "body"]
        }),
    }]
}
