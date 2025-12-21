/// Timeout utility functions for parsing and applying timeouts
use serverless_workflow_core::models::duration::OneOfDurationOrIso8601Expression;
use serverless_workflow_core::models::timeout::OneOfTimeoutDefinitionOrReference;
use std::time::Duration as StdDuration;

use super::Error;

pub type Result<T> = std::result::Result<T, Error>;

/// Parse ISO 8601 duration string into std::time::Duration
///
/// Supports formats like:
/// - PT5S (5 seconds)
/// - PT1M (1 minute)
/// - PT1M30S (1 minute 30 seconds)
/// - PT0.5S (0.5 seconds = 500ms)
/// - PT0.05M (0.05 minutes = 3 seconds)
fn parse_iso8601_duration(iso_str: &str) -> Result<StdDuration> {
    let trimmed = iso_str.trim();

    if !trimmed.starts_with('P') {
        return Err(Error::TaskExecution {
            message: format!("Invalid ISO 8601 duration: must start with 'P', got: {iso_str}"),
        });
    }

    let without_p = &trimmed[1..];

    if !without_p.starts_with('T') {
        return Err(Error::TaskExecution {
            message: format!(
                "ISO 8601 date components not yet supported, only time components (PT...) are supported, got: {iso_str}"
            ),
        });
    }

    let time_part = &without_p[1..];

    if time_part.is_empty() {
        return Err(Error::TaskExecution {
            message: format!(
                "Invalid ISO 8601 duration: no time components specified, got: {iso_str}"
            ),
        });
    }

    let mut total_ms: f64 = 0.0;
    let mut current_num = String::new();

    for ch in time_part.chars() {
        if ch.is_ascii_digit() || ch == '.' {
            current_num.push(ch);
        } else {
            if current_num.is_empty() {
                return Err(Error::TaskExecution {
                    message: format!("Invalid ISO 8601 duration format: {iso_str}"),
                });
            }

            let value: f64 = current_num.parse().map_err(|_| Error::TaskExecution {
                message: format!("Failed to parse number in ISO 8601 duration: {current_num}"),
            })?;

            match ch {
                'H' => total_ms += value * 3600.0 * 1000.0, // hours to ms
                'M' => total_ms += value * 60.0 * 1000.0,    // minutes to ms
                'S' => total_ms += value * 1000.0,           // seconds to ms
                _ => {
                    return Err(Error::TaskExecution {
                        message: format!("Unsupported ISO 8601 time unit: {ch}"),
                    });
                }
            }

            current_num.clear();
        }
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(StdDuration::from_millis(total_ms as u64))
}

/// Parse a timeout definition into a std::time::Duration
pub fn parse_timeout_duration(
    timeout: &OneOfTimeoutDefinitionOrReference,
) -> Result<StdDuration> {
    match timeout {
        OneOfTimeoutDefinitionOrReference::Timeout(def) => match &def.after {
            OneOfDurationOrIso8601Expression::Duration(d) => {
                let millis = d.total_milliseconds();
                Ok(StdDuration::from_millis(millis))
            }
            OneOfDurationOrIso8601Expression::Iso8601Expression(iso_str) => {
                parse_iso8601_duration(iso_str)
            }
        },
        OneOfTimeoutDefinitionOrReference::Reference(_ref_str) => {
            // TODO: Support timeout references from workflow.timeouts map
            Err(Error::Configuration {
                message: "Timeout references not yet supported".to_string(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serverless_workflow_core::models::duration::Duration;
    use serverless_workflow_core::models::timeout::TimeoutDefinition;

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
    fn test_parse_iso8601_complex() {
        let duration = parse_iso8601_duration("PT1M30S").unwrap();
        assert_eq!(duration.as_secs(), 90);
    }

    #[test]
    fn test_parse_iso8601_fractional() {
        let duration = parse_iso8601_duration("PT0.5S").unwrap();
        assert_eq!(duration.as_millis(), 500);
    }

    #[test]
    fn test_parse_timeout_inline_duration() {
        let timeout = OneOfTimeoutDefinitionOrReference::Timeout(TimeoutDefinition {
            after: OneOfDurationOrIso8601Expression::Duration(Duration::from_seconds(10)),
        });
        let duration = parse_timeout_duration(&timeout).unwrap();
        assert_eq!(duration.as_secs(), 10);
    }

    #[test]
    fn test_parse_timeout_iso8601() {
        let timeout = OneOfTimeoutDefinitionOrReference::Timeout(TimeoutDefinition {
            after: OneOfDurationOrIso8601Expression::Iso8601Expression("PT15S".to_string()),
        });
        let duration = parse_timeout_duration(&timeout).unwrap();
        assert_eq!(duration.as_secs(), 15);
    }
}
