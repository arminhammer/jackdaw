// `Status` is #[non_exhaustive]: a match outside jackdaw needs a wildcard arm.
use jackdaw::api::Status;

fn describe(value: &Status) -> &'static str {
    match value {
        Status::Running => "running",
        Status::Completed => "completed",
        Status::Faulted => "faulted",
        Status::Cancelled => "cancelled",
    }
}

fn main() {
    let _ = describe;
}
