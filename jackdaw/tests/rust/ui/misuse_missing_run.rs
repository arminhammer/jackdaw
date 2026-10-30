// Every workflow has exactly one `#[run]` method.
use jackdaw::prelude::*;

#[workflow(namespace = "ui", name = "missing-run", version = "1.0.0")]
pub struct W;

#[workflow_methods]
impl W {
    fn helper(&self) -> u32 {
        1
    }
}

fn main() {}
