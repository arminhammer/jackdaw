// `cache::Error` is #[non_exhaustive]: a custom cache's match outside jackdaw needs a wildcard arm.
use jackdaw::cache::Error;

fn describe(value: &Error) -> &'static str {
    match value {
        Error::Cache { .. } => "cache",
        Error::Serialization { .. } => "serialization",
        Error::Database { .. } => "database",
    }
}

fn main() {
    let _ = describe;
}
