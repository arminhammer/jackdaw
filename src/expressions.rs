use regex::Regex;
use serde_json::Value;
use snafu::prelude::*;

use jaq_core::Ctx;
use tracing::debug;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Expression evaluation error: {message}"))]
    Evaluation { message: String },

    #[snafu(display("JQ load errors: {errors:?}"))]
    JqLoad { errors: String },

    #[snafu(display("JQ compile errors: {errors:?}"))]
    JqCompile { errors: String },

    #[snafu(display("JQ evaluation error: {message}"))]
    JqEvaluation { message: String },
}

pub type Result<T> = std::result::Result<T, Error>;

/// Handles preprocessing of jq expressions to add null-safe operations
///
/// This preprocessor applies transformations to make jq expressions more robust
/// by automatically adding null-safe operators in common patterns.
#[derive(Debug)]
pub struct ExpressionPreprocessor {
    /// Enable null-safe field access transformations
    null_safe_enabled: bool,
}

impl Default for ExpressionPreprocessor {
    fn default() -> Self {
        Self::new()
    }
}

impl ExpressionPreprocessor {
    /// Create a new preprocessor with null-safe transformations enabled
    #[must_use]
    pub const fn new() -> Self {
        Self {
            null_safe_enabled: true,
        }
    }

    /// Preprocess a jq expression with configured transformations
    ///
    /// # Panics
    ///
    /// Panics if regex compilation fails (should not happen with hardcoded valid regex patterns).
    #[must_use]
    pub fn preprocess(&self, expr: &str) -> String {
        let mut result = expr.to_string();

        if self.null_safe_enabled {
            // Extract string literals to prevent transforming them
            let (expr_without_strings, strings) = self.extract_strings(expr);

            // Apply transformations only to non-string parts
            let mut transformed = self.apply_null_safe_field_access(&expr_without_strings);
            transformed = self.apply_null_safe_array_ops(&transformed);

            // Restore string literals
            result = self.restore_strings(&transformed, &strings);
        }

        result
    }

    /// Extract string literals from expression, replacing them with placeholders
    ///
    /// Returns the expression with placeholders and a list of extracted strings
    fn extract_strings(&self, expr: &str) -> (String, Vec<String>) {
        let mut strings = Vec::new();
        let mut result = String::new();
        let mut chars = expr.chars().peekable();
        let mut in_string = false;
        let mut escape_next = false;
        let mut current_string = String::new();

        while let Some(ch) = chars.next() {
            if escape_next {
                if in_string {
                    current_string.push('\\');
                    current_string.push(ch);
                } else {
                    result.push('\\');
                    result.push(ch);
                }
                escape_next = false;
                continue;
            }

            if ch == '\\' {
                escape_next = true;
                continue;
            }

            if ch == '"' {
                if in_string {
                    // End of string
                    strings.push(current_string.clone());
                    result.push_str(&format!("__STRING_PLACEHOLDER_{}__", strings.len() - 1));
                    current_string.clear();
                    in_string = false;
                } else {
                    // Start of string
                    in_string = true;
                }
            } else if in_string {
                current_string.push(ch);
            } else {
                result.push(ch);
            }
        }

        (result, strings)
    }

    /// Restore string literals from placeholders
    fn restore_strings(&self, expr: &str, strings: &[String]) -> String {
        let mut result = expr.to_string();
        for (i, s) in strings.iter().enumerate() {
            result = result.replace(
                &format!("__STRING_PLACEHOLDER_{i}__"),
                &format!("\"{s}\"")
            );
        }
        result
    }

    /// Apply null-safe transformations to nested field access patterns
    ///
    /// Transforms `.parent.child` into `(.parent // {}).child` to prevent
    /// errors when parent is null/missing.
    ///
    /// # Panics
    ///
    /// Panics if regex compilation fails (should not happen with hardcoded valid regex patterns).
    #[must_use]
    fn apply_null_safe_field_access(&self, expr: &str) -> String {
        // Transform: .parent.child -> (.parent // {}).child
        // This ensures that if .parent is null/missing, we get an empty object
        // instead of a jq error
        let re_parent =
            Regex::new(r"(\.[a-zA-Z_][a-zA-Z0-9_]*)\.([a-zA-Z_][a-zA-Z0-9_]*)").unwrap();
        re_parent
            .replace_all(expr, |caps: &regex::Captures| {
                format!("({} // {}).{}", &caps[1], "{}", &caps[2])
            })
            .to_string()
    }

    /// Apply null-safe transformations to array addition patterns
    ///
    /// Transforms `(.field + [x])` into `((.field // []) + [x])` to prevent
    /// errors when the field is null/missing.
    ///
    /// # Panics
    ///
    /// Panics if regex compilation fails (should not happen with hardcoded valid regex patterns).
    #[must_use]
    fn apply_null_safe_array_ops(&self, expr: &str) -> String {
        // Transform: (.field + [...]) -> ((.field // []) + [...])
        // This ensures that if .field is null/missing, we treat it as an empty array
        // before appending new elements
        let re = Regex::new(r"\((\.[a-zA-Z_][a-zA-Z0-9_]*(?:\.[a-zA-Z0-9_]*)*)\s*\+\s*\[").unwrap();
        re.replace_all(expr, |caps: &regex::Captures| {
            format!("(({} // []) + [", &caps[1])
        })
        .to_string()
    }
}

/// Evaluates an expression with the given context.
///
/// # Errors
///
/// Returns an error if expression evaluation fails or if jq compilation/execution encounters an error.
pub fn evaluate_expression(expression: &str, context: &Value) -> Result<Value> {
    evaluate_expression_with_input(expression, context, &Value::Null)
}

/// Evaluates an expression with access to both context and input values.
///
/// # Errors
///
/// Returns an error if expression evaluation fails or if jq compilation/execution encounters an error.
///
/// # Panics
///
/// Panics if regex compilation fails (should not happen with hardcoded valid regex patterns).
pub fn evaluate_expression_with_input(
    expression: &str,
    context: &Value,
    input: &Value,
) -> Result<Value> {
    let expr = expression.trim();
    if !expr.starts_with("${") || !expr.ends_with('}') {
        return Ok(Value::String(expression.to_string()));
    }

    let jq_expr_raw = expr[2..expr.len() - 1].trim();

    // Apply null-safe transformations using preprocessor
    // This handles patterns like:
    // - .parent.child -> (.parent // {}).child
    // - (.field + [...]) -> ((.field // []) + [...])
    let preprocessor = ExpressionPreprocessor::new();
    let mut jq_expr = preprocessor.preprocess(jq_expr_raw);

    // According to spec: when input is provided, expressions evaluate against $input, not context
    // This means . refers to $input in task contexts
    let has_input = !input.is_null();

    // Build evaluation context and bind variables
    // We need to detect which $variables are used and bind them using jaq's 'as' syntax
    let eval_context = if has_input {
        // When we have input, evaluate expressions against input
        // But we still need to bind context variables like $workflow, $runtime, etc.
        // Special handling: $input needs to reference the input itself
        let stripped_input = strip_descriptors(input);

        if let Some(context_obj) = context.as_object() {
            let mut var_bindings: Vec<(String, Value)> = Vec::new();

            // Detect all $varname references in the expression
            let var_regex = Regex::new(r"\$([a-zA-Z_][a-zA-Z0-9_]*)").unwrap();
            // Collect variable names first to avoid lifetime issues
            let var_names: Vec<String> = var_regex
                .captures_iter(&jq_expr)
                .map(|cap| cap[1].to_string())
                .collect();

            let has_input_ref = var_names.contains(&"input".to_string());
            let has_workflow_ref = var_names.contains(&"workflow".to_string());
            let has_runtime_ref = var_names.contains(&"runtime".to_string());

            // If $input is referenced, we need to wrap and bind it
            // Otherwise, . already refers to input
            if has_input_ref || has_workflow_ref || has_runtime_ref || var_names.iter().any(|v| v != "input" && v != "workflow" && v != "runtime" && context_obj.contains_key(v)) {
                // Build wrapper object with all needed bindings
                let mut wrapper = serde_json::Map::new();
                wrapper.insert("__value".to_string(), stripped_input.clone());

                if has_input_ref {
                    wrapper.insert("input".to_string(), stripped_input.clone());
                    var_bindings.push(("input".to_string(), Value::Null)); // Placeholder
                }

                if has_workflow_ref {
                    if let Some(workflow_desc) = context_obj.get("__workflow").cloned() {
                        wrapper.insert("workflow".to_string(), workflow_desc);
                        var_bindings.push(("workflow".to_string(), Value::Null));
                    }
                }

                if has_runtime_ref {
                    if let Some(runtime_desc) = context_obj.get("__runtime").cloned() {
                        wrapper.insert("runtime".to_string(), runtime_desc);
                        var_bindings.push(("runtime".to_string(), Value::Null));
                    }
                }

                // Bind other variables from context
                for var_name in var_names {
                    if var_name != "input" && var_name != "workflow" && var_name != "runtime" {
                        if let Some(val) = context_obj.get(&var_name) {
                            wrapper.insert(var_name.clone(), val.clone());
                            if !var_bindings.iter().any(|(n, _)| n == &var_name) {
                                var_bindings.push((var_name, Value::Null));
                            }
                        }
                    }
                }

                // Build bindings and wrap expression
                let binding_exprs: Vec<String> = var_bindings
                    .iter()
                    .map(|(name, _)| format!(".{name} as ${name}"))
                    .collect();

                jq_expr = format!("{} | .__value | {}", binding_exprs.join(" | "), jq_expr);
                Value::Object(wrapper)
            } else {
                // No special variables, just evaluate against input
                stripped_input
            }
        } else {
            stripped_input
        }
    } else if let Some(obj) = context.as_object() {
        // No input provided, fall back to old behavior (evaluate against context)
        let mut combined = obj.clone();
        let mut var_bindings = Vec::new();

        // Handle $input
        if jq_expr.contains("$input") {
            combined.insert("input".to_string(), strip_descriptors(input));
            var_bindings.push("input".to_string());
        }

        // Handle $workflow
        if jq_expr.contains("$workflow") {
            if let Some(workflow_desc) = combined.get("__workflow").cloned() {
                combined.insert("workflow".to_string(), workflow_desc);
            }
            var_bindings.push("workflow".to_string());
        }

        // Handle $runtime
        if jq_expr.contains("$runtime") {
            if let Some(runtime_desc) = combined.get("__runtime").cloned() {
                combined.insert("runtime".to_string(), runtime_desc);
            }
            var_bindings.push("runtime".to_string());
        }

        // Detect all $varname references
        let var_regex = Regex::new(r"\$([a-zA-Z_][a-zA-Z0-9_]*)").unwrap();
        for cap in var_regex.captures_iter(&jq_expr.clone()) {
            let var_name = &cap[1];
            if combined.contains_key(var_name) && !var_bindings.contains(&var_name.to_string()) {
                var_bindings.push(var_name.to_string());
            }
        }

        // Build the variable bindings
        if !var_bindings.is_empty() {
            let bindings: Vec<String> = var_bindings
                .iter()
                .map(|v| format!(".{v} as ${v}"))
                .collect();
            jq_expr = format!("{} | {}", bindings.join(" | "), jq_expr);
        }

        Value::Object(combined)
    } else {
        context.clone()
    };

    debug!("  Evaluating jq expression: {}", jq_expr);

    evaluate_jq(&jq_expr, &eval_context)
}

/// Evaluates a jq expression on a value (used for output filtering)
///
/// # Errors
///
/// Returns an error if jq compilation/execution encounters an error.
#[allow(dead_code)]
pub fn evaluate_jq_expression(jq_expr: &str, value: &Value) -> Result<Value> {
    evaluate_jq(jq_expr, value)
}

/// Evaluates a jq expression with access to $input variable (used for output.as expressions)
///
/// # Errors
///
/// Returns an error if jq compilation/execution encounters an error.
pub fn evaluate_jq_expression_with_context(
    jq_expr: &str,
    value: &Value,
    context: &Value,
) -> Result<Value> {
    // If the expression uses $input, we need to bind it
    if jq_expr.contains("$input") {
        // Strip descriptors from context when used as $input
        let input_value = strip_descriptors(context);

        // Wrap both value and input in an object so we can bind them
        let mut wrapper = serde_json::Map::new();
        wrapper.insert("__value".to_string(), value.clone());
        wrapper.insert("__input".to_string(), input_value);

        // Bind $input, then evaluate expression on the value
        let modified_expr = format!(".__input as $input | .__value | {jq_expr}");

        evaluate_jq(&modified_expr, &Value::Object(wrapper))
    } else {
        // No $input, just evaluate directly
        evaluate_jq(jq_expr, value)
    }
}

/// Evaluates a jq expression directly without requiring ${ } wrapper
///
/// # Errors
///
/// Returns an error if jq compilation/execution encounters an error.
pub fn evaluate_jq(jq_expr: &str, context: &Value) -> Result<Value> {
    use jaq_core::{
        compile::Compiler,
        load::{Arena, File, Loader},
    };

    // Create arena and loader with standard library (including jaq-json defs)
    let arena = Arena::default();
    let loader = Loader::new(jaq_std::defs().chain(jaq_json::defs()));

    // Load the expression as a "file"
    let file: File<_, ()> = File {
        path: (),
        code: jq_expr,
    };

    let modules = loader.load(&arena, file).map_err(|errs| Error::JqLoad {
        errors: format!("{errs:?}"),
    })?;

    // Compile with standard library native functions (including jaq-json funs)
    let compiler = Compiler::default().with_funs(jaq_std::funs().chain(jaq_json::funs()));
    let filter = compiler.compile(modules).map_err(|errs| Error::JqCompile {
        errors: format!("{errs:?}"),
    })?;

    // Convert serde_json::Value to jaq_json::Val using From trait
    let input: jaq_json::Val = context.clone().into();

    let inputs = jaq_core::RcIter::new(core::iter::empty());
    let mut results: Vec<_> = filter.run((Ctx::new([], &inputs), input)).collect();

    if results.is_empty() {
        return Ok(Value::Null);
    }

    match results.remove(0) {
        Ok(val) => {
            let result: serde_json::Value = val.into();
            Ok(result)
        }
        Err(e) => Err(Error::JqEvaluation {
            message: format!("{e}"),
        }),
    }
}

/// Remove internal descriptor fields from a value (used for $input in output.as expressions)
#[must_use]
pub fn strip_descriptors(value: &Value) -> Value {
    if let Value::Object(obj) = value {
        let mut cleaned = obj.clone();
        cleaned.remove("__workflow");
        cleaned.remove("__runtime");
        Value::Object(cleaned)
    } else {
        value.clone()
    }
}

/// Evaluates a value recursively, processing any expression strings found.
///
/// # Errors
///
/// Returns an error if expression evaluation fails or if jq compilation/execution encounters an error.
#[allow(dead_code)]
pub fn evaluate_value(value: &Value, context: &Value) -> Result<Value> {
    evaluate_value_with_input(value, context, &Value::Null)
}

/// Evaluates a value recursively with input, processing any expression strings found.
///
/// # Errors
///
/// Returns an error if expression evaluation fails or if jq compilation/execution encounters an error.
#[allow(dead_code)]
pub fn evaluate_value_with_input(value: &Value, context: &Value, input: &Value) -> Result<Value> {
    match value {
        Value::String(s) => evaluate_expression_with_input(s, context, input),
        Value::Object(map) => {
            let mut result = serde_json::Map::new();
            for (k, v) in map {
                result.insert(k.clone(), evaluate_value_with_input(v, context, input)?);
            }
            Ok(Value::Object(result))
        }
        Value::Array(arr) => {
            let mut result = Vec::new();
            for item in arr {
                result.push(evaluate_value_with_input(item, context, input)?);
            }
            Ok(Value::Array(result))
        }
        other => Ok(other.clone()),
    }
}
