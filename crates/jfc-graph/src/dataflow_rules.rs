//! Per-language rules for dataflow extraction.
//!
//! Each language defines which tree-sitter node kinds map to dataflow
//! constructs (parameters, assignments, calls, field accesses, mutations).

/// Language-specific rules for dataflow extraction.
pub struct DataflowRules {
    /// Node kinds representing function definitions.
    pub function_nodes: &'static [&'static str],
    /// Field name for the parameter list of a function.
    pub param_list_field: &'static str,
    /// Node kind for a single parameter identifier.
    pub param_identifier: &'static str,
    /// Field name for the function body.
    pub body_field: &'static str,
    /// Node kind for return statements/expressions.
    pub return_node: &'static str,
    /// Node kinds representing assignment or variable declaration.
    pub assignment_nodes: &'static [&'static str],
    /// Field name for the left-hand side of an assignment.
    pub assign_left_field: &'static str,
    /// Field name for the right-hand side of an assignment.
    pub assign_right_field: &'static str,
    /// Node kinds representing call expressions.
    pub call_nodes: &'static [&'static str],
    /// Field name for the function being called.
    pub call_function_field: &'static str,
    /// Field name for the arguments list of a call.
    pub call_args_field: &'static str,
    /// Node kind for member/field access expressions.
    pub member_node: &'static str,
    /// Field name for the object in a member expression.
    pub member_object_field: &'static str,
    /// Field name for the property/field in a member expression.
    pub member_property_field: &'static str,
    /// Method names that mutate their receiver.
    pub mutating_methods: &'static [&'static str],
    /// Node kind for plain identifiers.
    pub identifier_node: &'static str,
    /// Node kinds representing literal values.
    pub literal_nodes: &'static [&'static str],
    /// Node kind for method call expressions (language-specific).
    /// Empty string if the language uses the same call_nodes for methods.
    pub method_call_node: &'static str,
    /// Field name for the method call receiver object.
    pub method_call_object_field: &'static str,
    /// Field name for the method name in a method call.
    pub method_call_name_field: &'static str,
    /// Field name for the arguments list in a method call.
    pub method_call_args_field: &'static str,
}

impl DataflowRules {
    /// Look up rules for a language by its identifier.
    pub fn for_language(lang: &str) -> Option<&'static DataflowRules> {
        match lang {
            "rust" => Some(&RUST_DATAFLOW_RULES),
            "typescript" | "javascript" => Some(&TYPESCRIPT_DATAFLOW_RULES),
            "python" => Some(&PYTHON_DATAFLOW_RULES),
            "go" => Some(&GO_DATAFLOW_RULES),
            _ => None,
        }
    }
}

// ─── Rust ────────────────────────────────────────────────────────────────────

static RUST_DATAFLOW_RULES: DataflowRules = DataflowRules {
    function_nodes: &["function_item"],
    param_list_field: "parameters",
    param_identifier: "identifier",
    body_field: "body",
    return_node: "return_expression",
    assignment_nodes: &["let_declaration"],
    assign_left_field: "pattern",
    assign_right_field: "value",
    call_nodes: &["call_expression"],
    call_function_field: "function",
    call_args_field: "arguments",
    member_node: "field_expression",
    member_object_field: "value",
    member_property_field: "field",
    mutating_methods: &[
        "push",
        "pop",
        "insert",
        "remove",
        "clear",
        "sort",
        "retain",
        "extend",
        "drain",
        "truncate",
        "sort_by",
        "sort_unstable",
        "append",
        "reserve",
    ],
    identifier_node: "identifier",
    literal_nodes: &[
        "integer_literal",
        "float_literal",
        "string_literal",
        "raw_string_literal",
        "char_literal",
        "boolean_literal",
    ],
    method_call_node: "call_expression",
    method_call_object_field: "",
    method_call_name_field: "",
    method_call_args_field: "arguments",
};

// ─── TypeScript ──────────────────────────────────────────────────────────────

static TYPESCRIPT_DATAFLOW_RULES: DataflowRules = DataflowRules {
    function_nodes: &[
        "function_declaration",
        "method_definition",
        "arrow_function",
        "function",
    ],
    param_list_field: "parameters",
    param_identifier: "identifier",
    body_field: "body",
    return_node: "return_statement",
    assignment_nodes: &["variable_declarator", "assignment_expression"],
    assign_left_field: "name",
    assign_right_field: "value",
    call_nodes: &["call_expression"],
    call_function_field: "function",
    call_args_field: "arguments",
    member_node: "member_expression",
    member_object_field: "object",
    member_property_field: "property",
    mutating_methods: &[
        "push", "pop", "shift", "unshift", "splice", "sort", "reverse", "fill",
    ],
    identifier_node: "identifier",
    literal_nodes: &[
        "number",
        "string",
        "true",
        "false",
        "null",
        "undefined",
        "template_string",
    ],
    method_call_node: "call_expression",
    method_call_object_field: "",
    method_call_name_field: "",
    method_call_args_field: "arguments",
};

// ─── Python ──────────────────────────────────────────────────────────────────

static PYTHON_DATAFLOW_RULES: DataflowRules = DataflowRules {
    function_nodes: &["function_definition"],
    param_list_field: "parameters",
    param_identifier: "identifier",
    body_field: "body",
    return_node: "return_statement",
    assignment_nodes: &["assignment", "augmented_assignment"],
    assign_left_field: "left",
    assign_right_field: "right",
    call_nodes: &["call"],
    call_function_field: "function",
    call_args_field: "arguments",
    member_node: "attribute",
    member_object_field: "object",
    member_property_field: "attribute",
    mutating_methods: &[
        "append", "extend", "insert", "remove", "pop", "clear", "sort", "reverse",
    ],
    identifier_node: "identifier",
    literal_nodes: &["integer", "float", "string", "true", "false", "none"],
    method_call_node: "call",
    method_call_object_field: "",
    method_call_name_field: "",
    method_call_args_field: "arguments",
};

// ─── Go ──────────────────────────────────────────────────────────────────────

static GO_DATAFLOW_RULES: DataflowRules = DataflowRules {
    function_nodes: &["function_declaration", "method_declaration"],
    param_list_field: "parameters",
    param_identifier: "identifier",
    body_field: "body",
    return_node: "return_statement",
    assignment_nodes: &["short_var_declaration", "assignment_statement"],
    assign_left_field: "left",
    assign_right_field: "right",
    call_nodes: &["call_expression"],
    call_function_field: "function",
    call_args_field: "arguments",
    member_node: "selector_expression",
    member_object_field: "operand",
    member_property_field: "field",
    mutating_methods: &["append", "delete", "Reset", "Write", "Close"],
    identifier_node: "identifier",
    literal_nodes: &[
        "int_literal",
        "float_literal",
        "rune_literal",
        "interpreted_string_literal",
        "raw_string_literal",
        "true",
        "false",
        "nil",
    ],
    method_call_node: "call_expression",
    method_call_object_field: "",
    method_call_name_field: "",
    method_call_args_field: "arguments",
};
