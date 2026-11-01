// `blocking` is for synchronous functions.
use jackdaw::prelude::*;

#[task(blocking)]
async fn grind(n: u64) -> Result<u64, TaskError> {
    Ok(n)
}

fn main() {}
