//! YAML workflows on the engine: `for`, `set`, `fork` and `call` of a Rust task in one, `switch` in another.
use jackdaw::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Serialize, Deserialize)]
struct Item {
    sku: String,
}

#[task(name = "examples.price")]
async fn price(item: Item) -> Result<u64, TaskError> {
    Ok(item.sku.len() as u64 * 10)
}

const ORDERS: &str = include_str!("../../orders.sw.yaml");
const SWITCH: &str = include_str!("../../switch.sw.yaml");

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let orders = Workflow::<Value, Value>::from_yaml(ORDERS)?;
    let switch = Workflow::<Value, Value>::from_yaml(SWITCH)?;
    let engine = Engine::builder()
        .task(Price)
        .yaml_workflow(&orders)
        .yaml_workflow(&switch)
        .build()?;
    let totalled = engine
        .execute(&orders, json!({ "items": [10, 20, 30] }))
        .await?
        .await?;
    println!("orders={totalled}");
    let classified = engine.execute(&switch, json!({ "n": 60 })).await?.await?;
    println!("switch={classified}");
    Ok(())
}
