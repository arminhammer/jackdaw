// `Error` is #[non_exhaustive]: a match outside jackdaw needs a wildcard arm.
use jackdaw::api::Error;

fn describe(value: &Error) -> &'static str {
    match value {
        Error::Faulted(_) => "faulted",
        Error::Cancelled => "cancelled",
        Error::Timeout { .. } => "timeout",
        Error::Invalid(_) => "invalid",
        Error::NonDeterminism(_) => "diverged",
        Error::Persistence(_) => "store",
        Error::Cache(_) => "cache",
        Error::Engine(_) => "engine",
        Error::Rejected(_) => "rejected",
    }
}

fn main() {
    let _ = describe;
}
