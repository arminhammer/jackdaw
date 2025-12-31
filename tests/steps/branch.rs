#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::expect_fun_call)]

use crate::CtKWorld;

pub use cucumber::{World, then};
pub use jackdaw::cache::CacheProvider;
pub use serde_json::Value;

#[then(expr = "the workflow output should have a {string} property containing {int} items")]
async fn then_property_contains_items(
    world: &mut CtKWorld,
    property_path: String,
    expected_count: usize,
) {
    let output = world
        .workflow_output
        .as_ref()
        .expect("No workflow output found");

    // Navigate to the property (handle nested paths like 'foo.bar.baz')
    let parts: Vec<&str> = property_path.split('.').collect();
    let mut current = output;

    for part in &parts {
        current = current.get(part).expect(&format!(
            "Property '{}' not found in path '{}'",
            part, property_path
        ));
    }

    // Check if it's an array and count items
    let actual_count = match current {
        Value::Array(arr) => arr.len(),
        _ => panic!(
            "Property '{}' is not an array, found: {:?}",
            property_path, current
        ),
    };

    assert_eq!(
        actual_count, expected_count,
        "Expected property '{}' to contain {} items, but found {}",
        property_path, expected_count, actual_count
    );
}
