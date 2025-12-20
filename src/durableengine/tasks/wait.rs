use crate::context::Context;
use crate::durableengine::DurableEngine;
use serverless_workflow_core::models::duration::OneOfDurationOrIso8601Expression;
use serverless_workflow_core::models::task::WaitTaskDefinition;
use std::time::Duration as StdDuration;

use super::Result;

/// Parse ISO 8601 duration string into std::time::Duration
///
/// Supports formats like:
/// - PT5S (5 seconds)
/// - PT1M (1 minute)
/// - PT1M30S (1 minute 30 seconds)
/// - PT0.5S (0.5 seconds = 500ms)
/// - PT0.05M (0.05 minutes = 3 seconds)
fn parse_iso8601_duration(iso_str: &str) -> Result<StdDuration> {
    // Simple ISO 8601 duration parser for PT format
    // Full spec: https://en.wikipedia.org/wiki/ISO_8601#Durations

    let trimmed = iso_str.trim();

    if !trimmed.starts_with('P') {
        return Err(crate::durableengine::Error::TaskExecution {
            message: format!("Invalid ISO 8601 duration: must start with 'P', got: {}", iso_str),
        });
    }

    // Remove 'P' prefix
    let without_p = &trimmed[1..];

    // Check if it contains time components (starts with T after P)
    if !without_p.starts_with('T') {
        // Date components not yet supported (would need to handle days, months, years)
        // For now, we only support time components
        return Err(crate::durableengine::Error::TaskExecution {
            message: format!("ISO 8601 date components not yet supported, only time components (PT...) are supported, got: {}", iso_str),
        });
    }

    // Remove 'T' prefix
    let time_part = &without_p[1..];

    // Empty duration (just "PT") is invalid
    if time_part.is_empty() {
        return Err(crate::durableengine::Error::TaskExecution {
            message: format!("Invalid ISO 8601 duration: no time components specified, got: {}", iso_str),
        });
    }

    let mut total_ms: f64 = 0.0;
    let mut current_num = String::new();

    for ch in time_part.chars() {
        if ch.is_ascii_digit() || ch == '.' {
            current_num.push(ch);
        } else {
            if current_num.is_empty() {
                return Err(crate::durableengine::Error::TaskExecution {
                    message: format!("Invalid ISO 8601 duration format: {}", iso_str),
                });
            }

            let value: f64 = current_num.parse().map_err(|_| {
                crate::durableengine::Error::TaskExecution {
                    message: format!("Failed to parse number in ISO 8601 duration: {}", current_num),
                }
            })?;

            match ch {
                'H' => total_ms += value * 3600.0 * 1000.0,  // hours to ms
                'M' => total_ms += value * 60.0 * 1000.0,     // minutes to ms
                'S' => total_ms += value * 1000.0,            // seconds to ms
                _ => {
                    return Err(crate::durableengine::Error::TaskExecution {
                        message: format!("Unsupported ISO 8601 time unit: {}", ch),
                    });
                }
            }

            current_num.clear();
        }
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(StdDuration::from_millis(total_ms as u64))
}

/// Execute a wait task
///
/// Waits for the specified duration before continuing workflow execution
pub async fn exec_wait_task(
    _engine: &DurableEngine,
    _task_name: &str,
    wait_task: &WaitTaskDefinition,
    _ctx: &Context,
) -> Result<serde_json::Value> {
    // Parse the duration
    let duration = match &wait_task.wait {
        OneOfDurationOrIso8601Expression::Duration(d) => {
            // Convert Duration to tokio::time::Duration using total_milliseconds
            let millis = d.total_milliseconds();
            StdDuration::from_millis(millis)
        }
        OneOfDurationOrIso8601Expression::Iso8601Expression(iso_str) => {
            // Parse ISO 8601 string
            parse_iso8601_duration(iso_str)?
        }
    };

    // Wait for the specified duration
    tokio::time::sleep(duration).await;

    // Return empty result (wait tasks don't produce output)
    Ok(serde_json::json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_iso8601_seconds() {
        let duration = parse_iso8601_duration("PT5S").unwrap();
        assert_eq!(duration.as_secs(), 5);
    }

    #[test]
    fn test_parse_iso8601_minutes() {
        let duration = parse_iso8601_duration("PT2M").unwrap();
        assert_eq!(duration.as_secs(), 120);
    }

    #[test]
    fn test_parse_iso8601_hours() {
        let duration = parse_iso8601_duration("PT1H").unwrap();
        assert_eq!(duration.as_secs(), 3600);
    }

    #[test]
    fn test_parse_iso8601_composite() {
        let duration = parse_iso8601_duration("PT1H30M15S").unwrap();
        assert_eq!(duration.as_secs(), 3600 + 1800 + 15);
    }

    #[test]
    fn test_parse_iso8601_fractional_seconds() {
        let duration = parse_iso8601_duration("PT0.5S").unwrap();
        assert_eq!(duration.as_millis(), 500);
    }

    #[test]
    fn test_parse_iso8601_fractional_minutes() {
        let duration = parse_iso8601_duration("PT0.05M").unwrap();
        assert_eq!(duration.as_millis(), 3000); // 0.05 minutes = 3 seconds
    }

    #[test]
    fn test_parse_iso8601_invalid() {
        assert!(parse_iso8601_duration("5S").is_err());
        assert!(parse_iso8601_duration("PT").is_err());
        assert!(parse_iso8601_duration("P5D").is_err()); // Date components not supported yet
    }
}