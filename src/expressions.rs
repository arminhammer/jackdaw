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
            result = self.apply_null_safe_field_access(&result);
            result = self.apply_null_safe_array_ops(&result);
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
        let re = Regex::new(r"\((\.[a-zA-Z_][a-zA-Z0-9_]*(?:\.[a-zA-Z0-9_]*)*)\s*\+\s*\[")
            .unwrap();
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

    // Build evaluation context and bind variables
    // We need to detect which $variables are used and bind them using jaq's 'as' syntax
    let eval_context = if let Some(obj) = context.as_object() {
        let mut combined = obj.clone();
        let mut var_bindings = Vec::new();

        // Handle $input
        if jq_expr.contains("$input") {
            // Strip internal descriptors from $input as they shouldn't be part of data flow
            combined.insert("input".to_string(), strip_descriptors(input));
            var_bindings.push("input".to_string());
        }

        // Handle $workflow - check if workflow descriptor is in context
        if jq_expr.contains("$workflow") {
            if let Some(workflow_desc) = combined.get("__workflow").cloned() {
                combined.insert("workflow".to_string(), workflow_desc);
            }
            var_bindings.push("workflow".to_string());
        }

        // Handle $runtime - check if runtime descriptor is in context
        if jq_expr.contains("$runtime") {
            if let Some(runtime_desc) = combined.get("__runtime").cloned() {
                combined.insert("runtime".to_string(), runtime_desc);
            }
            var_bindings.push("runtime".to_string());
        }

        // Detect all $varname references in the expression
        let var_regex = Regex::new(r"\$([a-zA-Z_][a-zA-Z0-9_]*)").unwrap();
        for cap in var_regex.captures_iter(&jq_expr.clone()) {
            let var_name = &cap[1];
            // Only bind if the variable exists in context and we haven't already added it
            if combined.contains_key(var_name) && !var_bindings.contains(&var_name.to_string()) {
                var_bindings.push(var_name.to_string());
            }
        }

        // Build the variable bindings at the start of the expression
        // Format: .varname as $varname | .var2 as $var2 | <original expression>
        if !var_bindings.is_empty() {
            let bindings: Vec<String> = var_bindings
                .iter()
                .map(|v| format!(".{v} as ${v}"))
                .collect();
            jq_expr = format!("{} | {}", bindings.join(" | "), jq_expr);
        }

        Value::Object(combined)
    } else {
        // Context is not an object (e.g., a string after input filtering)
        // Keep it as-is and special variables will be handled via jaq vars if needed
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
