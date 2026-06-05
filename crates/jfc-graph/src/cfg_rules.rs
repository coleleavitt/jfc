//! Per-language rules for control-flow graph construction.
//!
//! Each language defines which tree-sitter node kinds map to control-flow
//! constructs (conditionals, loops, switches, jumps, exceptions).

/// Language-specific rules for CFG construction.
pub struct CfgRules {
    /// Node kinds representing if/elif/conditional expressions.
    pub if_nodes: &'static [&'static str],
    /// Node kind for else clauses.
    pub else_node: Option<&'static str>,
    /// Node kinds for for-style loops.
    pub for_nodes: &'static [&'static str],
    /// Node kinds for while-style loops.
    pub while_nodes: &'static [&'static str],
    /// Node kind for infinite loops (Rust's `loop`).
    pub loop_node: Option<&'static str>,
    /// Node kinds for switch/match statements.
    pub switch_nodes: &'static [&'static str],
    /// Node kinds for individual case/arm entries.
    pub case_nodes: &'static [&'static str],
    /// Node kinds for try blocks.
    pub try_nodes: &'static [&'static str],
    /// Node kind for catch/except clauses.
    pub catch_node: Option<&'static str>,
    /// Node kind for finally blocks.
    pub finally_node: Option<&'static str>,
    /// Node kind for return statements/expressions.
    pub return_node: Option<&'static str>,
    /// Node kind for break statements.
    pub break_node: Option<&'static str>,
    /// Node kind for continue statements.
    pub continue_node: Option<&'static str>,
    /// Node kind for throw/raise/panic expressions.
    pub throw_node: Option<&'static str>,
    /// Field name to extract the function body.
    pub body_field: &'static str,
    /// Node kinds representing function definitions.
    pub function_nodes: &'static [&'static str],
}

impl CfgRules {
    /// Look up rules for a language by its identifier.
    pub fn for_language(lang: &str) -> Option<&'static CfgRules> {
        match lang {
            "rust" => Some(&RUST_CFG_RULES),
            "typescript" | "javascript" => Some(&TYPESCRIPT_CFG_RULES),
            "python" => Some(&PYTHON_CFG_RULES),
            "go" => Some(&GO_CFG_RULES),
            "java" => Some(&JAVA_CFG_RULES),
            "c" => Some(&C_CFG_RULES),
            "cpp" => Some(&CPP_CFG_RULES),
            "php" => Some(&PHP_CFG_RULES),
            _ => None,
        }
    }
}

// ─── Rust ────────────────────────────────────────────────────────────────────

static RUST_CFG_RULES: CfgRules = CfgRules {
    if_nodes: &["if_expression"],
    else_node: Some("else_clause"),
    for_nodes: &["for_expression"],
    while_nodes: &["while_expression"],
    loop_node: Some("loop_expression"),
    switch_nodes: &["match_expression"],
    case_nodes: &["match_arm"],
    try_nodes: &[],
    catch_node: None,
    finally_node: None,
    return_node: Some("return_expression"),
    break_node: Some("break_expression"),
    continue_node: Some("continue_expression"),
    throw_node: None,
    body_field: "body",
    function_nodes: &["function_item"],
};

// ─── TypeScript ──────────────────────────────────────────────────────────────

static TYPESCRIPT_CFG_RULES: CfgRules = CfgRules {
    if_nodes: &["if_statement"],
    else_node: Some("else_clause"),
    for_nodes: &["for_statement", "for_in_statement"],
    while_nodes: &["while_statement", "do_statement"],
    loop_node: None,
    switch_nodes: &["switch_statement"],
    case_nodes: &["switch_case", "switch_default"],
    try_nodes: &["try_statement"],
    catch_node: Some("catch_clause"),
    finally_node: Some("finally_clause"),
    return_node: Some("return_statement"),
    break_node: Some("break_statement"),
    continue_node: Some("continue_statement"),
    throw_node: Some("throw_statement"),
    body_field: "body",
    function_nodes: &[
        "function_declaration",
        "method_definition",
        "arrow_function",
    ],
};

// ─── Python ──────────────────────────────────────────────────────────────────

static PYTHON_CFG_RULES: CfgRules = CfgRules {
    if_nodes: &["if_statement"],
    else_node: Some("else_clause"),
    for_nodes: &["for_statement"],
    while_nodes: &["while_statement"],
    loop_node: None,
    switch_nodes: &["match_statement"],
    case_nodes: &["case_clause"],
    try_nodes: &["try_statement"],
    catch_node: Some("except_clause"),
    finally_node: Some("finally_clause"),
    return_node: Some("return_statement"),
    break_node: Some("break_statement"),
    continue_node: Some("continue_statement"),
    throw_node: Some("raise_statement"),
    body_field: "body",
    function_nodes: &["function_definition"],
};

// ─── Go ──────────────────────────────────────────────────────────────────────

static GO_CFG_RULES: CfgRules = CfgRules {
    if_nodes: &["if_statement"],
    else_node: Some("else_clause"),
    for_nodes: &["for_statement"],
    while_nodes: &[],
    loop_node: None,
    switch_nodes: &["expression_switch_statement", "type_switch_statement"],
    case_nodes: &["expression_case", "type_case", "default_case"],
    try_nodes: &[],
    catch_node: None,
    finally_node: None,
    return_node: Some("return_statement"),
    break_node: Some("break_statement"),
    continue_node: Some("continue_statement"),
    throw_node: None,
    body_field: "body",
    function_nodes: &["function_declaration", "method_declaration"],
};

// ─── Java ────────────────────────────────────────────────────────────────────

static JAVA_CFG_RULES: CfgRules = CfgRules {
    if_nodes: &["if_statement"],
    else_node: Some("else_clause"),
    for_nodes: &["for_statement", "enhanced_for_statement"],
    while_nodes: &["while_statement", "do_statement"],
    loop_node: None,
    switch_nodes: &["switch_expression"],
    case_nodes: &["switch_block_statement_group", "switch_rule"],
    try_nodes: &["try_statement"],
    catch_node: Some("catch_clause"),
    finally_node: Some("finally_clause"),
    return_node: Some("return_statement"),
    break_node: Some("break_statement"),
    continue_node: Some("continue_statement"),
    throw_node: Some("throw_statement"),
    body_field: "body",
    function_nodes: &["method_declaration", "constructor_declaration"],
};

// ─── C ───────────────────────────────────────────────────────────────────────

static C_CFG_RULES: CfgRules = CfgRules {
    if_nodes: &["if_statement"],
    else_node: Some("else_clause"),
    for_nodes: &["for_statement"],
    while_nodes: &["while_statement", "do_statement"],
    loop_node: None,
    switch_nodes: &["switch_statement"],
    case_nodes: &["case_statement"],
    try_nodes: &[],
    catch_node: None,
    finally_node: None,
    return_node: Some("return_statement"),
    break_node: Some("break_statement"),
    continue_node: Some("continue_statement"),
    throw_node: None,
    body_field: "body",
    function_nodes: &["function_definition"],
};

// ─── C++ ─────────────────────────────────────────────────────────────────────

static CPP_CFG_RULES: CfgRules = CfgRules {
    if_nodes: &["if_statement"],
    else_node: Some("else_clause"),
    for_nodes: &["for_statement", "for_range_loop"],
    while_nodes: &["while_statement", "do_statement"],
    loop_node: None,
    switch_nodes: &["switch_statement"],
    case_nodes: &["case_statement"],
    try_nodes: &["try_statement"],
    catch_node: Some("catch_clause"),
    finally_node: None,
    return_node: Some("return_statement"),
    break_node: Some("break_statement"),
    continue_node: Some("continue_statement"),
    throw_node: Some("throw_statement"),
    body_field: "body",
    function_nodes: &["function_definition"],
};

// ─── PHP ─────────────────────────────────────────────────────────────────────

static PHP_CFG_RULES: CfgRules = CfgRules {
    if_nodes: &["if_statement"],
    else_node: Some("else_clause"),
    for_nodes: &["for_statement", "foreach_statement"],
    while_nodes: &["while_statement", "do_statement"],
    loop_node: None,
    switch_nodes: &["switch_statement", "match_expression"],
    case_nodes: &[
        "case_statement",
        "default_statement",
        "match_conditional_expression",
    ],
    try_nodes: &["try_statement"],
    catch_node: Some("catch_clause"),
    finally_node: Some("finally_clause"),
    return_node: Some("return_statement"),
    break_node: Some("break_statement"),
    continue_node: Some("continue_statement"),
    throw_node: Some("throw_expression"),
    body_field: "body",
    function_nodes: &[
        "function_definition",
        "method_declaration",
        "arrow_function",
    ],
};
