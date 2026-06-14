use jfc_provider::ToolDef;

pub fn design_tool_defs() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "DesignProjectCreate".into(),
            description: "Create a persistent design project under `.jfc/design/projects/<id>/`.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Human-readable project title." }
                },
                "required": ["title"]
            }),
        },
        ToolDef {
            name: "DesignProjectList".into(),
            description: "List persistent JFC design projects and their registered deliverable assets.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        ToolDef {
            name: "DesignProjectSetMeta".into(),
            description: "Update a design project's title and/or design-system flag.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "project_id": { "type": "string" },
                    "title": { "type": "string" },
                    "is_design_system": { "type": "boolean" }
                },
                "required": ["project_id"]
            }),
        },
        ToolDef {
            name: "DesignListFiles".into(),
            description: "List project-relative files in a design project sandbox.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "project_id": { "type": "string" }
                },
                "required": ["project_id"]
            }),
        },
        ToolDef {
            name: "DesignReadFile".into(),
            description: "Read a project-relative file from a design project sandbox.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "project_id": { "type": "string" },
                    "path": { "type": "string", "description": "Project-relative path." }
                },
                "required": ["project_id", "path"]
            }),
        },
        ToolDef {
            name: "DesignWriteFile".into(),
            description: "Write a project-relative file in a design project sandbox, optionally registering it as a deliverable asset.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "project_id": { "type": "string" },
                    "path": { "type": "string", "description": "Project-relative destination path." },
                    "content": { "type": "string" },
                    "asset_name": { "type": "string", "description": "Optional deliverable asset name to register for this path." }
                },
                "required": ["project_id", "path", "content"]
            }),
        },
        ToolDef {
            name: "DesignDeleteFile".into(),
            description: "Delete a project-relative file or directory from a design project sandbox.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "project_id": { "type": "string" },
                    "path": { "type": "string" }
                },
                "required": ["project_id", "path"]
            }),
        },
        ToolDef {
            name: "DesignCopyFile".into(),
            description: "Copy one project-relative file to another path inside a design project sandbox.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "project_id": { "type": "string" },
                    "from_path": { "type": "string" },
                    "to_path": { "type": "string" }
                },
                "required": ["project_id", "from_path", "to_path"]
            }),
        },
        ToolDef {
            name: "DesignRegisterAsset".into(),
            description: "Register a design project file as a named deliverable asset.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "project_id": { "type": "string" },
                    "name": { "type": "string" },
                    "path": { "type": "string" }
                },
                "required": ["project_id", "name", "path"]
            }),
        },
        ToolDef {
            name: "DesignUnregisterAsset".into(),
            description: "Remove a project-relative file from the design project's deliverable asset registry.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "project_id": { "type": "string" },
                    "path": { "type": "string" }
                },
                "required": ["project_id", "path"]
            }),
        },
        ToolDef {
            name: "DesignBundleHtml".into(),
            description: "Native `super_inline_html`: inline local CSS, JS, image, media, and CSS url() assets into one standalone HTML file.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "input": { "type": "string", "description": "HTML file path." },
                    "output": { "type": "string", "description": "Output HTML path. Defaults to input with .standalone.html-like extension." },
                    "require_thumbnail": { "type": "boolean", "description": "Require a __bundler_thumbnail template before bundling. Defaults true." }
                },
                "required": ["input"]
            }),
        },
        ToolDef {
            name: "DesignHandoff".into(),
            description: "Create a Claude-Code-style design handoff directory with a README skeleton and copied design files.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "project_dir": { "type": "string" },
                    "feature": { "type": "string" },
                    "files": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Files relative to project_dir, or absolute paths, to copy into the handoff bundle."
                    }
                },
                "required": ["project_dir", "feature"]
            }),
        },
        ToolDef {
            name: "DesignCheckSystem".into(),
            description: "Index and validate a JFC design system, writing `_ds_manifest.json` and reporting tokens, fonts, components, cards, and starting points.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "project_dir": { "type": "string" }
                },
                "required": ["project_dir"]
            }),
        },
        ToolDef {
            name: "DesignCapabilities".into(),
            description: "Show the Claude Design to JFC parity matrix as text, markdown, or JSON.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "format": { "type": "string", "enum": ["text", "markdown", "json"] }
                },
                "required": []
            }),
        },
        ToolDef {
            name: "DesignServe".into(),
            description: "Start a detached localhost preview server for a design directory and return the preview URL. This is the native `show_html`/`show_to_user` preview foundation; browser eval/screenshots come in the browser-host phase.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "project_dir": { "type": "string", "description": "Directory to serve." },
                    "port": { "type": "number", "description": "Port to bind. Defaults to an available OS-assigned port." },
                    "file": { "type": "string", "description": "Optional project-relative file to include in the returned URL." }
                },
                "required": ["project_dir"]
            }),
        },
    ]
}
