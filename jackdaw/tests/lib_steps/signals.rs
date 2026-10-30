//! tasks/state_and_signals.feature
use super::support::*;
use super::world::{Fixture, LibWorld};
use cucumber::{given, then, when};
use jackdaw::prelude::*;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

pub struct Shop(pub Arc<Probe>);

#[tasks]
impl Shop {
    #[task(name = "cart.place_order")]
    pub async fn place_order(
        self: Arc<Self>,
        ctx: TaskContext,
        items: Vec<String>,
    ) -> Result<usize, TaskError> {
        self.0.typed("place_order", &ctx, &items).await
    }
}

pub fn defaults(probe: &Probe) {
    probe.default_output("place_order", |items| {
        json!(items.as_array().map_or(0, Vec::len))
    });
}

const KNOWN: &[&str] = &["A", "B", "C", "welcome-gift"];

#[workflow(namespace = "cart", name = "cart", version = "1")]
pub struct Cart {
    items: Vec<String>,
    checked_out: bool,
}

#[workflow_methods]
impl Cart {
    #[init]
    fn new(_ctx: &WorkflowContextView) -> Self {
        Self {
            items: Vec::new(),
            checked_out: false,
        }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<usize> {
        ctx.wait_condition(|cart| cart.checked_out).await?;
        let items = ctx.state(|cart| cart.items.clone());
        Ok(ctx.call(Shop::place_order, &items).await?)
    }

    #[signal]
    fn add_item(&mut self, _ctx: &mut SyncWorkflowContext<Self>, sku: String) {
        self.items.push(sku);
    }

    #[signal]
    fn remove_item(&mut self, _ctx: &mut SyncWorkflowContext<Self>, sku: String) {
        self.items.retain(|item| *item != sku);
    }

    #[signal]
    fn checkout(&mut self, _ctx: &mut SyncWorkflowContext<Self>, _: ()) {
        self.checked_out = true;
    }

    #[update]
    fn add_known(
        &mut self,
        _ctx: &mut SyncWorkflowContext<Self>,
        sku: String,
    ) -> Result<usize, UpdateError> {
        if !KNOWN.contains(&sku.as_str()) {
            return Err(UpdateError::new(format!("unknown sku `{sku}`")));
        }
        self.items.push(sku);
        Ok(self.items.len())
    }

    #[query]
    fn items(&self, _ctx: &WorkflowContextView) -> Vec<String> {
        self.items.clone()
    }
}

/// A cart that starts with a gift.
#[workflow(namespace = "cart", name = "gift-cart", version = "1")]
pub struct GiftCart {
    items: Vec<String>,
}

#[workflow_methods]
impl GiftCart {
    #[init]
    fn new(_ctx: &WorkflowContextView) -> Self {
        Self {
            items: vec!["welcome-gift".into()],
        }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<()> {
        ctx.wait_condition(|_| false).await?;
        Ok(())
    }

    #[query]
    fn items(&self, _ctx: &WorkflowContextView) -> Vec<String> {
        self.items.clone()
    }
}

#[workflow(namespace = "cart", name = "abandonable", version = "1")]
pub struct Abandonable {
    checked_out: bool,
}

#[workflow_methods]
impl Abandonable {
    #[init]
    fn new(_ctx: &WorkflowContextView) -> Self {
        Self { checked_out: false }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<String> {
        Ok(jackdaw::select! {
            _ = ctx.wait_condition(|c| c.checked_out) => "checked out".to_string(),
            _ = ctx.sleep(7.days()) => "abandon".to_string(),
        })
    }

    #[signal]
    fn checkout(&mut self, _ctx: &mut SyncWorkflowContext<Self>, _: ()) {
        self.checked_out = true;
    }
}

pub fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture::of::<Cart>("cart.cart"),
        Fixture::of::<GiftCart>("cart.gift"),
        Fixture::of::<Abandonable>("cart.abandonable"),
    ]
}

#[given(regex = r#"^a workflow "Cart" whose state is a list of items and a "checked_out" flag$"#)]
fn cart(world: &mut LibWorld) {
    world.set_fixture(super::fixture("cart.cart"));
}

#[given(
    regex = r#"^"\#\[signal\] add_item", "\#\[signal\] remove_item" and "\#\[signal\] checkout" handlers$"#
)]
fn cart_signals(_world: &mut LibWorld) {}

#[given(regex = r#"^a "\#\[query\] items" handler$"#)]
fn cart_query(_world: &mut LibWorld) {}

#[given("a run method that does:")]
fn cart_run(_world: &mut LibWorld) {}

#[given(regex = r#"^a running "Cart"$"#)]
async fn running_cart(world: &mut LibWorld) {
    world
        .start_fixture("it", super::fixture("cart.cart"), json!(null))
        .await;
}

async fn signal(world: &mut LibWorld, name: &str, sku: &str) {
    let handle = world.handle(world.last());
    match name {
        "add_item" => handle.signal(Cart::add_item, sku.to_string()).await,
        "remove_item" => handle.signal(Cart::remove_item, sku.to_string()).await,
        "checkout" => handle.signal(Cart::checkout, ()).await,
        other => panic!("no signal {other}"),
    }
    .expect("signal");
}

#[when(regex = r#"^I signal "add_item" with "A", "add_item" with "B" and "remove_item" with "A"$"#)]
async fn signal_three(world: &mut LibWorld) {
    signal(world, "add_item", "A").await;
    signal(world, "add_item", "B").await;
    signal(world, "remove_item", "A").await;
}

#[when(regex = r#"^I signal "checkout"$"#)]
async fn signal_checkout(world: &mut LibWorld) {
    signal(world, "checkout", "").await;
}

#[then(regex = r#"^"place_order" ran with (\[.*\])$"#)]
async fn place_order_with(world: &mut LibWorld, items: String) {
    let last = world.last();
    world.outcome(last).await.value();
    let expected: Value = serde_json::from_str(&items).expect("JSON");
    assert_eq!(world.probe.calls_of("place_order")[0].input, expected);
}

#[given(regex = r#"^a running "Cart" that received "add_item" with "(\w+)"$"#)]
async fn cart_with(world: &mut LibWorld, sku: String) {
    world
        .start_fixture("it", super::fixture("cart.cart"), json!(null))
        .await;
    signal(world, "add_item", &sku).await;
}

#[when(regex = r#"^I query "items"$"#)]
async fn query_items(world: &mut LibWorld) {
    let handle = world.handle(world.last());
    let items = if world.runs[world.last()]
        .fixture
        .as_ref()
        .is_some_and(|f| f.name == "cart.gift")
    {
        handle.query(GiftCart::items).await
    } else {
        handle.query(Cart::items).await
    }
    .expect("query");
    world.notes.insert("answer".into(), json!(items));
}

#[then(regex = r"^the answer is (\[.*\])$")]
fn answer_is(world: &mut LibWorld, expected: String) {
    let expected: Value = serde_json::from_str(&expected).expect("JSON");
    assert_eq!(world.notes["answer"], expected);
}

#[given(regex = r#"^"Cart" has an "\#\[init\]" that starts with the item "welcome-gift"$"#)]
fn gift_cart(world: &mut LibWorld) {
    world.set_fixture(super::fixture("cart.gift"));
}

#[when(regex = r#"^I execute it and query "items"$"#)]
async fn execute_and_query(world: &mut LibWorld) {
    world.start("it", json!(null)).await;
    query_items(world).await;
}

#[given(regex = r#"^a running "Cart" that received "add_item" A, "add_item" B, "remove_item" A$"#)]
async fn cart_three(world: &mut LibWorld) {
    world
        .start_fixture("it", super::fixture("cart.cart"), json!(null))
        .await;
    signal_three(world).await;
}

#[given(regex = r#"^a run method that does "jackdaw::select! \{ _ = ctx\.wait_condition.+"$"#)]
fn abandonable(world: &mut LibWorld) {
    world.set_fixture(super::fixture("cart.abandonable"));
}

#[when(regex = r#"^(\d+) days pass without "checkout"$"#)]
async fn days_pass(world: &mut LibWorld, days: u64) {
    world.start("it", json!(null)).await;
    world.pass(Duration::from_secs(days * 86_400)).await;
}

#[then("the workflow takes the abandon branch")]
async fn abandon_branch(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.value(), &json!("abandon"));
}

#[given(regex = r#"^a "\#\[signal\]" handler that tries to call "ctx\.call"$"#)]
fn signal_calls(world: &mut LibWorld) {
    world
        .notes
        .insert("ui".into(), json!("misuse_signal_calls_task"));
}

#[then(regex = r#"^it fails because "(\w+)" has no "(\w+)"$"#)]
fn fails_because(world: &mut LibWorld, ty: String, method: String) {
    let stderr = world.notes["stderr"].as_str().expect("compiled");
    assert!(
        stderr.contains(&format!("no method named `{method}`")) && stderr.contains(&ty),
        "{stderr}"
    );
}

#[given(regex = r#"^a "\#\[update\] add_item\(.+" that rejects unknown skus$"#)]
async fn update_cart(world: &mut LibWorld) {
    world
        .start_fixture("it", super::fixture("cart.cart"), json!(null))
        .await;
}

#[when("I send the update with a known sku")]
async fn update_known(world: &mut LibWorld) {
    let handle = world.handle(world.last());
    let count = handle
        .update(Cart::add_known, "A".to_string())
        .await
        .expect("accepted");
    world.notes.insert("update".into(), json!(count));
}

#[then("the caller receives the new item count")]
fn new_count(world: &mut LibWorld) {
    assert_eq!(world.notes["update"], json!(1));
}

#[when("I send the update with an unknown sku")]
async fn update_unknown(world: &mut LibWorld) {
    let handle = world.handle(world.last());
    let rejected = handle
        .update(Cart::add_known, "Z".to_string())
        .await
        .expect_err("rejected");
    world
        .notes
        .insert("update".into(), json!(rejected.to_string()));
}

#[then("the caller receives the rejection and the state is unchanged")]
async fn rejected_unchanged(world: &mut LibWorld) {
    let message = world.notes["update"]
        .as_str()
        .expect("an error")
        .to_string();
    assert!(message.contains("unknown sku"), "{message}");
    let handle = world.handle(world.last());
    assert_eq!(
        handle.query(Cart::items).await.expect("query"),
        vec!["A".to_string()]
    );
}
