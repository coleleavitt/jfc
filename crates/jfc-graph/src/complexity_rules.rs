//! Per-language rules for complexity analysis.
//!
//! Each language defines which tree-sitter node kinds map to branches,
//! nesting structures, logical operators, case nodes, and Halstead
//! operator/operand categories.

/// Language-specific rules for complexity computation.
pub struct LangRules {
    /// Node kinds that represent breaks in linear flow (if, for, while, etc.).
    /// Contribute +1 to both cognitive and cyclomatic complexity.
    pub branch_nodes: &'static [&'static str],

    /// Node kinds that represent individual case/arm entries in a match/switch.
    /// Contribute +1 to cyclomatic complexity (not cognitive).
    pub case_nodes: &'static [&'static str],

    /// Node kinds that can contain a logical operator in their "operator" field.
    /// The actual operator text is checked against `logical_operators`.
    pub logical_op_nodes: &'static [&'static str],

    /// Operator text strings that count as logical operators (e.g. "&&", "||").
    pub logical_operators: &'static [&'static str],

    /// Node kinds that increase nesting depth (for cognitive nesting penalty).
    pub nesting_nodes: &'static [&'static str],

    /// Node kinds that represent the function itself (used for body extraction).
    pub function_nodes: &'static [&'static str],

    /// Field names to check for the function body node.
    pub body_field_names: &'static [&'static str],

    /// Node kinds counted as operators in Halstead metrics.
    pub operator_nodes: &'static [&'static str],

    /// Node kinds counted as operands in Halstead metrics.
    pub operand_nodes: &'static [&'static str],

    /// Node kinds that contain an "operator" field child which should be
    /// counted as an operator (e.g. binary_expression, unary_expression).
    pub operator_container_nodes: &'static [&'static str],
}

impl LangRules {
    /// Look up rules for a language by its identifier.
    pub fn for_language(language_id: &str) -> Option<&'static LangRules> {
        match language_id {
            "rust" => Some(&RUST_RULES),
            "typescript" => Some(&TYPESCRIPT_RULES),
            "python" => Some(&PYTHON_RULES),
            "go" => Some(&GO_RULES),
            "java" => Some(&JAVA_RULES),
            "c" => Some(&C_RULES),
            "cpp" => Some(&CPP_RULES),
            "php" => Some(&PHP_RULES),
            "kotlin" => Some(&KOTLIN_RULES),
            "swift" => Some(&SWIFT_RULES),
            "csharp" => Some(&CSHARP_RULES),
            "ruby" => Some(&RUBY_RULES),
            _ => None,
        }
    }
}

// ─── Rust ────────────────────────────────────────────────────────────────────

static RUST_RULES: LangRules = LangRules {
    branch_nodes: &[
        "if_expression",
        "match_expression",
        "for_expression",
        "while_expression",
        "loop_expression",
    ],
    case_nodes: &["match_arm"],
    logical_op_nodes: &["binary_expression"],
    logical_operators: &["&&", "||"],
    nesting_nodes: &[
        "if_expression",
        "match_expression",
        "for_expression",
        "while_expression",
        "loop_expression",
        "closure_expression",
    ],
    function_nodes: &["function_item"],
    body_field_names: &["body"],
    operator_nodes: &[
        "return_expression",
        "if_expression",
        "match_expression",
        "for_expression",
        "while_expression",
        "loop_expression",
        "let_declaration",
    ],
    operand_nodes: &[
        "identifier",
        "integer_literal",
        "float_literal",
        "string_literal",
        "boolean_literal",
        "char_literal",
    ],
    operator_container_nodes: &[
        "binary_expression",
        "unary_expression",
        "compound_assignment_expr",
        "assignment_expression",
    ],
};

// ─── TypeScript ──────────────────────────────────────────────────────────────

static TYPESCRIPT_RULES: LangRules = LangRules {
    branch_nodes: &[
        "if_statement",
        "switch_statement",
        "for_statement",
        "for_in_statement",
        "while_statement",
        "do_statement",
        "ternary_expression",
        "catch_clause",
    ],
    case_nodes: &["switch_case", "switch_default"],
    logical_op_nodes: &["binary_expression"],
    logical_operators: &["&&", "||", "??"],
    nesting_nodes: &[
        "if_statement",
        "switch_statement",
        "for_statement",
        "for_in_statement",
        "while_statement",
        "do_statement",
        "try_statement",
        "arrow_function",
    ],
    function_nodes: &[
        "function_declaration",
        "method_definition",
        "arrow_function",
    ],
    body_field_names: &["body"],
    operator_nodes: &[
        "return_statement",
        "if_statement",
        "switch_statement",
        "for_statement",
        "while_statement",
        "do_statement",
        "variable_declaration",
        "lexical_declaration",
    ],
    operand_nodes: &[
        "identifier",
        "number",
        "string",
        "template_string",
        "true",
        "false",
        "null",
        "undefined",
    ],
    operator_container_nodes: &[
        "binary_expression",
        "unary_expression",
        "assignment_expression",
        "augmented_assignment_expression",
    ],
};

// ─── Python ──────────────────────────────────────────────────────────────────

static PYTHON_RULES: LangRules = LangRules {
    branch_nodes: &[
        "if_statement",
        "for_statement",
        "while_statement",
        "match_statement",
        "except_clause",
        "elif_clause",
    ],
    case_nodes: &["case_clause"],
    logical_op_nodes: &["boolean_operator"],
    logical_operators: &["and", "or"],
    nesting_nodes: &[
        "if_statement",
        "for_statement",
        "while_statement",
        "match_statement",
        "try_statement",
        "with_statement",
        "lambda",
    ],
    function_nodes: &["function_definition"],
    body_field_names: &["body"],
    operator_nodes: &[
        "return_statement",
        "if_statement",
        "for_statement",
        "while_statement",
        "assignment",
    ],
    operand_nodes: &[
        "identifier",
        "integer",
        "float",
        "string",
        "true",
        "false",
        "none",
    ],
    operator_container_nodes: &[
        "binary_operator",
        "unary_operator",
        "boolean_operator",
        "comparison_operator",
        "augmented_assignment",
    ],
};

// ─── Go ──────────────────────────────────────────────────────────────────────

static GO_RULES: LangRules = LangRules {
    branch_nodes: &[
        "if_statement",
        "for_statement",
        "expression_switch_statement",
        "type_switch_statement",
        "select_statement",
    ],
    case_nodes: &[
        "expression_case",
        "type_case",
        "default_case",
        "communication_case",
    ],
    logical_op_nodes: &["binary_expression"],
    logical_operators: &["&&", "||"],
    nesting_nodes: &[
        "if_statement",
        "for_statement",
        "expression_switch_statement",
        "type_switch_statement",
        "select_statement",
        "func_literal",
    ],
    function_nodes: &["function_declaration", "method_declaration"],
    body_field_names: &["body"],
    operator_nodes: &[
        "return_statement",
        "if_statement",
        "for_statement",
        "go_statement",
        "defer_statement",
        "short_var_declaration",
        "var_declaration",
    ],
    operand_nodes: &[
        "identifier",
        "int_literal",
        "float_literal",
        "rune_literal",
        "raw_string_literal",
        "interpreted_string_literal",
        "true",
        "false",
        "nil",
    ],
    operator_container_nodes: &[
        "binary_expression",
        "unary_expression",
        "assignment_statement",
    ],
};

// ─── Java ────────────────────────────────────────────────────────────────────

static JAVA_RULES: LangRules = LangRules {
    branch_nodes: &[
        "if_statement",
        "for_statement",
        "enhanced_for_statement",
        "while_statement",
        "do_statement",
        "catch_clause",
        "ternary_expression",
        "switch_expression",
    ],
    case_nodes: &["switch_block_statement_group", "switch_rule"],
    logical_op_nodes: &["binary_expression"],
    logical_operators: &["&&", "||"],
    nesting_nodes: &[
        "if_statement",
        "for_statement",
        "enhanced_for_statement",
        "while_statement",
        "do_statement",
        "try_statement",
        "switch_expression",
        "lambda_expression",
    ],
    function_nodes: &["method_declaration", "constructor_declaration"],
    body_field_names: &["body"],
    operator_nodes: &[
        "return_statement",
        "if_statement",
        "for_statement",
        "enhanced_for_statement",
        "while_statement",
        "do_statement",
        "throw_statement",
        "local_variable_declaration",
    ],
    operand_nodes: &[
        "identifier",
        "decimal_integer_literal",
        "hex_integer_literal",
        "octal_integer_literal",
        "binary_integer_literal",
        "decimal_floating_point_literal",
        "string_literal",
        "character_literal",
        "true",
        "false",
        "null_literal",
    ],
    operator_container_nodes: &[
        "binary_expression",
        "unary_expression",
        "assignment_expression",
        "update_expression",
    ],
};

// ─── C ───────────────────────────────────────────────────────────────────────

static C_RULES: LangRules = LangRules {
    branch_nodes: &[
        "if_statement",
        "for_statement",
        "while_statement",
        "do_statement",
        "switch_statement",
    ],
    case_nodes: &["case_statement"],
    logical_op_nodes: &["binary_expression"],
    logical_operators: &["&&", "||"],
    nesting_nodes: &[
        "if_statement",
        "for_statement",
        "while_statement",
        "do_statement",
        "switch_statement",
    ],
    function_nodes: &["function_definition"],
    body_field_names: &["body"],
    operator_nodes: &[
        "return_statement",
        "if_statement",
        "for_statement",
        "while_statement",
        "do_statement",
        "switch_statement",
        "goto_statement",
        "declaration",
    ],
    operand_nodes: &[
        "identifier",
        "number_literal",
        "string_literal",
        "char_literal",
        "true",
        "false",
        "null",
    ],
    operator_container_nodes: &[
        "binary_expression",
        "unary_expression",
        "assignment_expression",
        "update_expression",
    ],
};

// ─── C++ ─────────────────────────────────────────────────────────────────────
//
// Extends C rules with C++-specific constructs (try/catch, range-for, etc.).
// Same AST node names for if/for/while/switch since tree-sitter-cpp extends
// tree-sitter-c.

static CPP_RULES: LangRules = LangRules {
    branch_nodes: &[
        "if_statement",
        "for_statement",
        "for_range_loop",
        "while_statement",
        "do_statement",
        "switch_statement",
        "catch_clause",
        "conditional_expression",
    ],
    case_nodes: &["case_statement"],
    logical_op_nodes: &["binary_expression"],
    logical_operators: &["&&", "||"],
    nesting_nodes: &[
        "if_statement",
        "for_statement",
        "for_range_loop",
        "while_statement",
        "do_statement",
        "switch_statement",
        "try_statement",
        "lambda_expression",
    ],
    function_nodes: &["function_definition"],
    body_field_names: &["body"],
    operator_nodes: &[
        "return_statement",
        "if_statement",
        "for_statement",
        "for_range_loop",
        "while_statement",
        "do_statement",
        "switch_statement",
        "throw_statement",
        "new_expression",
        "delete_expression",
        "declaration",
    ],
    operand_nodes: &[
        "identifier",
        "number_literal",
        "string_literal",
        "char_literal",
        "true",
        "false",
        "nullptr",
        "this",
    ],
    operator_container_nodes: &[
        "binary_expression",
        "unary_expression",
        "assignment_expression",
        "update_expression",
        "conditional_expression",
    ],
};

// ─── PHP ─────────────────────────────────────────────────────────────────────

pub static PHP_RULES: LangRules = LangRules {
    branch_nodes: &[
        "if_statement",
        "else_clause",
        "for_statement",
        "foreach_statement",
        "while_statement",
        "do_statement",
        "catch_clause",
        "switch_statement",
        "match_expression",
    ],
    case_nodes: &[
        "case_statement",
        "default_statement",
        "match_conditional_expression",
    ],
    logical_op_nodes: &["binary_expression"],
    logical_operators: &["&&", "||", "and", "or", "??"],
    nesting_nodes: &[
        "if_statement",
        "for_statement",
        "foreach_statement",
        "while_statement",
        "do_statement",
        "catch_clause",
        "switch_statement",
        "match_expression",
    ],
    function_nodes: &[
        "function_definition",
        "method_declaration",
        "arrow_function",
    ],
    body_field_names: &["body"],
    operator_nodes: &[
        "if_statement",
        "for_statement",
        "foreach_statement",
        "while_statement",
        "do_statement",
        "switch_statement",
        "return_statement",
        "echo_statement",
        "throw_expression",
        "new_expression",
        "yield_expression",
    ],
    operand_nodes: &[
        "variable_name",
        "name",
        "integer",
        "float",
        "string",
        "boolean",
        "null",
        "encapsed_string",
    ],
    operator_container_nodes: &[
        "binary_expression",
        "unary_op_expression",
        "assignment_expression",
        "augmented_assignment_expression",
        "update_expression",
        "conditional_expression",
    ],
};

// ─── C# ──────────────────────────────────────────────────────────────────────

static CSHARP_RULES: LangRules = LangRules {
    branch_nodes: &[
        "if_statement",
        "for_statement",
        "for_each_statement",
        "while_statement",
        "do_statement",
        "catch_clause",
        "switch_statement",
        "switch_expression",
        "conditional_expression",
    ],
    case_nodes: &["switch_section", "switch_expression_arm"],
    logical_op_nodes: &["binary_expression"],
    logical_operators: &["&&", "||", "??"],
    nesting_nodes: &[
        "if_statement",
        "for_statement",
        "for_each_statement",
        "while_statement",
        "do_statement",
        "switch_statement",
        "switch_expression",
        "try_statement",
        "lambda_expression",
    ],
    function_nodes: &["method_declaration", "constructor_declaration"],
    body_field_names: &["body"],
    operator_nodes: &[
        "return_statement",
        "if_statement",
        "for_statement",
        "for_each_statement",
        "while_statement",
        "do_statement",
        "switch_statement",
        "throw_statement",
        "local_declaration_statement",
        "object_creation_expression",
    ],
    operand_nodes: &[
        "identifier",
        "integer_literal",
        "real_literal",
        "string_literal",
        "verbatim_string_literal",
        "interpolated_string_expression",
        "character_literal",
        "boolean_literal",
        "null_literal",
    ],
    operator_container_nodes: &[
        "binary_expression",
        "prefix_unary_expression",
        "postfix_unary_expression",
        "assignment_expression",
    ],
};

// ─── Ruby ────────────────────────────────────────────────────────────────────

static RUBY_RULES: LangRules = LangRules {
    branch_nodes: &[
        "if", "unless", "elsif", "when", "for", "while", "until", "rescue",
    ],
    case_nodes: &["when"],
    logical_op_nodes: &["binary"],
    logical_operators: &["and", "or", "&&", "||"],
    nesting_nodes: &["if", "unless", "for", "while", "until", "rescue", "case"],
    function_nodes: &["method", "singleton_method", "lambda", "block"],
    body_field_names: &["body"],
    operator_nodes: &[
        "return",
        "if",
        "unless",
        "for",
        "while",
        "until",
        "case",
        "assignment",
        "operator_assignment",
    ],
    operand_nodes: &[
        "identifier",
        "constant",
        "integer",
        "float",
        "string",
        "symbol",
        "true",
        "false",
        "nil",
    ],
    operator_container_nodes: &["binary", "unary", "assignment", "operator_assignment"],
};

// ─── Kotlin ──────────────────────────────────────────────────────────────────

pub static KOTLIN_RULES: LangRules = LangRules {
    branch_nodes: &[
        "if_expression",
        "when_expression",
        "for_statement",
        "while_statement",
        "do_while_statement",
        "catch_block",
    ],
    case_nodes: &["when_entry"],
    logical_op_nodes: &["conjunction_expression", "disjunction_expression"],
    logical_operators: &["&&", "||"],
    nesting_nodes: &[
        "if_expression",
        "when_expression",
        "for_statement",
        "while_statement",
        "do_while_statement",
        "catch_block",
    ],
    function_nodes: &[
        "function_declaration",
        "lambda_literal",
        "anonymous_function",
    ],
    body_field_names: &["body", "function_body"],
    operator_nodes: &[
        "if_expression",
        "for_statement",
        "while_statement",
        "do_while_statement",
        "when_expression",
        "return_expression",
        "throw_expression",
        "object_creation_expression",
    ],
    operand_nodes: &[
        "simple_identifier",
        "integer_literal",
        "real_literal",
        "string_literal",
        "boolean_literal",
        "null_literal",
    ],
    operator_container_nodes: &[
        "additive_expression",
        "multiplicative_expression",
        "comparison_expression",
        "equality_expression",
        "conjunction_expression",
        "disjunction_expression",
        "prefix_expression",
        "postfix_expression",
        "assignment",
    ],
};

// ─── Swift ───────────────────────────────────────────────────────────────────

pub static SWIFT_RULES: LangRules = LangRules {
    branch_nodes: &[
        "if_statement",
        "guard_statement",
        "for_statement",
        "while_statement",
        "repeat_while_statement",
        "switch_statement",
        "catch_clause",
    ],
    case_nodes: &["switch_case", "case_item"],
    logical_op_nodes: &["binary_expression"],
    logical_operators: &["&&", "||"],
    nesting_nodes: &[
        "if_statement",
        "guard_statement",
        "for_statement",
        "while_statement",
        "repeat_while_statement",
        "switch_statement",
        "catch_clause",
    ],
    function_nodes: &[
        "function_declaration",
        "init_declaration",
        "closure_expression",
    ],
    body_field_names: &["body", "function_body"],
    operator_nodes: &[
        "if_statement",
        "guard_statement",
        "for_statement",
        "while_statement",
        "repeat_while_statement",
        "switch_statement",
        "return_statement",
        "throw_statement",
    ],
    operand_nodes: &[
        "simple_identifier",
        "integer_literal",
        "real_literal",
        "string_literal",
        "boolean_literal",
        "nil",
    ],
    operator_container_nodes: &[
        "binary_expression",
        "prefix_expression",
        "postfix_expression",
        "assignment",
        "ternary_expression",
    ],
};

// ─── Kotlin ──────────────────────────────────────────────────────────────────
