use jaq_interpret::{FilterT, RcIter};
use serde_json::Value;
use snafu::prelude::*;
use std::rc::Rc;

use core::fmt::{self, Display, Formatter};
use jaq_core::{Ctx, Native, compile, load};
use jaq_json::Val;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Termination};
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

    // Build evaluation context
    // If context is an object, we can add special variables to it
    // If context is a scalar, we keep it as-is and pass special variables through jaq vars
    let eval_context = if let Some(obj) = context.as_object() {
        let mut combined = obj.clone();

        // Handle $input
        if jq_expr.contains("$input") {
            combined.insert("input".to_string(), input.clone());
            jq_expr = jq_expr.replace("$input", ".input");
        }

        // Handle $workflow - check if workflow descriptor is in context
        if jq_expr.contains("$workflow") {
            if let Some(workflow_desc) = combined.get("__workflow").cloned() {
                combined.insert("workflow".to_string(), workflow_desc);
            }
            jq_expr = jq_expr.replace("$workflow", ".workflow");
        }

        // Handle $runtime - check if runtime descriptor is in context
        if jq_expr.contains("$runtime") {
            if let Some(runtime_desc) = combined.get("__runtime").cloned() {
                combined.insert("runtime".to_string(), runtime_desc);
            }
            jq_expr = jq_expr.replace("$runtime", ".runtime");
        }

        // Replace $varname with .varname for variables that exist as top-level fields in context
        for key in combined.keys() {
            let var_ref = format!("${}", key);
            let field_ref = format!(".{}", key);
            jq_expr = jq_expr.replace(&var_ref, &field_ref);
        }

        Value::Object(combined)
    } else {
        // Context is not an object (e.g., a string after input filtering)
        // Keep it as-is and special variables will be handled via jaq vars if needed
        context.clone()
    };

    // Null-safe array operations: wrap field accesses before + with // []
    // This handles cases like: (.processed.colors + [x]) -> (((.processed // {}).colors // []) + [x])
    // First, wrap parent object accesses: .processed.colors -> (.processed // {}).colors
    use regex::Regex;
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

    debug!("  Evaluating jq expression: {}", jq_expr);

    let result = evaluate_jq(&jq_expr, &eval_context);
    if let Err(ref e) = result {
        println!("  Expression evaluation error: {}", e);
    }
    result
}

/// Evaluates a jq expression on a value (used for output filtering)
pub fn evaluate_jq_expression(jq_expr: &str, value: &Value) -> Result<Value> {
    evaluate_jq(jq_expr, value)
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
        errors: format!("{:?}", errs),
    })?;

    // Compile with standard library native functions (including jaq-json funs)
    let compiler = Compiler::default().with_funs(jaq_std::funs().chain(jaq_json::funs()));
    let filter = compiler.compile(modules).map_err(|errs| Error::JqCompile {
        errors: format!("{:?}", errs),
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
            message: format!("{}", e),
        }),
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

fn json_to_val(value: &Value) -> Val {
    match value {
        Value::Null => Val::Null,
        Value::Bool(b) => Val::Bool(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Val::Int(i as isize)
            } else if let Some(f) = n.as_f64() {
                Val::Float(f)
            } else {
                Val::Null
            }
        }
        Value::String(s) => Val::Str(Rc::from(s.clone())),
        Value::Array(arr) => Val::Arr(Rc::from(arr.iter().map(json_to_val).collect::<Vec<_>>())),
        Value::Object(obj) => {
            let map = obj
                .iter()
                .map(|(k, v)| (Rc::from(k.clone()), json_to_val(v)))
                .collect();
            Val::Obj(Rc::new(map))
        }
    }
}

fn val_to_json(val: &Val) -> Value {
    match val {
        Val::Null => Value::Null,
        Val::Bool(b) => Value::Bool(*b),
        Val::Int(i) => Value::Number((*i).into()),
        Val::Float(f) => serde_json::Number::from_f64(*f)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        Val::Num(s) => {
            if let Ok(i) = s.parse::<i64>() {
                Value::Number(i.into())
            } else if let Ok(f) = s.parse::<f64>() {
                serde_json::Number::from_f64(f)
                    .map(Value::Number)
                    .unwrap_or(Value::Null)
            } else {
                Value::String(s.to_string())
            }
        }
        Val::Str(s) => Value::String(s.to_string()),
        Val::Arr(arr) => Value::Array(arr.iter().map(val_to_json).collect()),
        Val::Obj(obj) => {
            let mut map = serde_json::Map::new();
            for (k, v) in obj.iter() {
                map.insert(k.to_string(), val_to_json(v));
            }
            Value::Object(map)
        }
    }
}
