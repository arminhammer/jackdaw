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

pub fn evaluate_expression(expression: &str, context: &Value) -> Result<Value> {
    evaluate_expression_with_input(expression, context, &Value::Null)
}

pub fn evaluate_expression_with_input(
    expression: &str,
    context: &Value,
    input: &Value,
) -> Result<Value> {
    let expr = expression.trim();
    if !expr.starts_with("${") || !expr.ends_with("}") {
        return Ok(Value::String(expression.to_string()));
    }

    let mut jq_expr = expr[2..expr.len() - 1].trim().to_string();

    // Null-safe array operations: wrap field accesses before + with // []
    // This handles cases like: (.processed.colors + [x]) -> (((.processed // {}).colors // []) + [x])
    // Do this BEFORE variable binding to avoid interfering with the binding syntax
    let re_parent = Regex::new(r"(\.[a-zA-Z_][a-zA-Z0-9_]*)\.([a-zA-Z_][a-zA-Z0-9_]*)").unwrap();
    jq_expr = re_parent
        .replace_all(&jq_expr, |caps: &regex::Captures| {
            format!("({} // {}).{}", &caps[1], "{}", &caps[2])
        })
        .to_string();

    // Then, wrap array additions with // []
    let re =
        Regex::new(r"\((\.[a-zA-Z_][a-zA-Z0-9_]*(?:\.[a-zA-Z_][a-zA-Z0-9_]*)*)\s*\+\s*\[").unwrap();
    jq_expr = re
        .replace_all(&jq_expr, |caps: &regex::Captures| {
            format!("(({} // []) + [", &caps[1])
        })
        .to_string();

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

    let result = evaluate_jq(&jq_expr, &eval_context);
    result
}

/// Evaluates a jq expression on a value (used for output filtering)
pub fn evaluate_jq_expression(jq_expr: &str, value: &Value) -> Result<Value> {
    evaluate_jq(jq_expr, value)
}

/// Evaluates a jq expression with access to $input variable (used for output.as expressions)
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

pub fn evaluate_value(value: &Value, context: &Value) -> Result<Value> {
    evaluate_value_with_input(value, context, &Value::Null)
}

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
