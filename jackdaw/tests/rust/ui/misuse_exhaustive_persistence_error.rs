// `persistence::Error` is #[non_exhaustive]: a custom store's match outside jackdaw needs a wildcard arm.
use jackdaw::persistence::Error;

fn describe(value: &Error) -> &'static str {
    match value {
        Error::Persistence { .. } => "store",
        Error::Database { .. } => "database",
        Error::Serialization { .. } => "serialization",
        Error::EventNotFound { .. } => "no events",
        Error::CheckpointNotFound { .. } => "no checkpoint",
    }
}

fn main() {
    let _ = describe;
}
