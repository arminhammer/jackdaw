// `executor::Error` is #[non_exhaustive]: a match outside jackdaw needs a wildcard arm.
use jackdaw::executor::Error;

fn describe(value: &Error) -> &'static str {
    match value {
        Error::Execution { .. } => "execution",
        Error::Task { .. } => "task",
    }
}

fn main() {
    let _ = describe;
}
