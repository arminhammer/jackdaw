//! `jackdaw::Error` is one matchable, well-behaved error type.
use crate::support::*;

fn assert_send_sync_static<T: Send + Sync + 'static>() {}

#[test]
fn error_and_fault_are_send_sync_and_static() {
    assert_send_sync_static::<Error>();
    assert_send_sync_static::<Fault>();
}

#[test]
fn error_converts_into_a_boxed_std_error() {
    let boxed: Box<dyn std::error::Error + Send + Sync> = Error::Cancelled.into();

    assert!(!boxed.to_string().is_empty());
}

#[test]
fn error_works_with_the_question_mark_in_a_function_returning_a_boxed_error() {
    fn parse() -> Result<Workflow<Name, Greeting>, Box<dyn std::error::Error>> {
        Ok(Workflow::from_yaml("not: [a workflow")?)
    }

    assert!(parse().is_err());
}

#[test]
fn invalid_yaml_is_an_invalid_error_with_a_helpful_message() {
    let err = Workflow::<Name, Greeting>::from_yaml("document: {}\ndo: nope").unwrap_err();

    assert!(matches!(err, Error::Invalid(_)), "got {err:?}");
    assert!(!err.to_string().is_empty());
}

#[test]
fn a_fault_displays_its_type_status_and_detail() {
    let fault = Fault {
        kind: error_types::COMMUNICATION.to_string(),
        status: Some(502),
        title: Some("Bad Gateway".into()),
        detail: Some("gateway down".into()),
        instance: None,
    };
    let text = Error::Faulted(fault).to_string();

    assert!(text.contains("502"), "{text}");
    assert!(text.contains("gateway down"), "{text}");
}

#[test]
fn error_types_are_the_specs_uris() {
    assert_eq!(
        error_types::COMMUNICATION,
        "https://open-workflow-specification.org/dsl/errors/types/communication"
    );
    assert_eq!(
        error_types::RUNTIME,
        "https://open-workflow-specification.org/dsl/errors/types/runtime"
    );
}

#[test]
fn match_arms_on_error_need_a_wildcard_because_it_can_grow() {
    // `Error` is #[non_exhaustive]: downstream matches must include `_`, so adding a variant is not a breaking change.
    let describe = |e: &Error| match e {
        Error::Cancelled => "cancelled",
        Error::Timeout { .. } => "timeout",
        Error::Faulted(_) => "fault",
        _ => "other",
    };

    assert_eq!(describe(&Error::Cancelled), "cancelled");
}
