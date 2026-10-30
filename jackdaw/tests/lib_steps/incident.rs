//! workflows/incident_response.feature
use super::support::*;
use super::world::{Fixture, LibWorld};
use cucumber::{given, then, when};
use jackdaw::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum Severity {
    Major,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Alert {
    pub service: String,
    pub severity: Severity,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Owner {
    pub escalation: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Resolution {
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Postmortem {
    pub duration_secs: i64,
    pub notes: Vec<String>,
    pub resolved: bool,
}

pub struct Ops(pub Arc<Probe>);

#[tasks]
impl Ops {
    #[task(name = "ops.diagnose")]
    pub async fn diagnose(
        self: Arc<Self>,
        ctx: TaskContext,
        alert: Alert,
    ) -> Result<String, TaskError> {
        self.0.typed("diagnose", &ctx, &alert).await
    }
    #[task(name = "ops.lookup_owner")]
    pub async fn lookup_owner(
        self: Arc<Self>,
        ctx: TaskContext,
        service: String,
    ) -> Result<Owner, TaskError> {
        self.0.typed("lookup_owner", &ctx, &service).await
    }
    #[task(name = "ops.page")]
    pub async fn page(
        self: Arc<Self>,
        ctx: TaskContext,
        who: (String, Alert),
    ) -> Result<(), TaskError> {
        self.0.typed("page", &ctx, &who).await
    }
    #[task(name = "ops.open_status_page")]
    pub async fn open_status_page(
        self: Arc<Self>,
        ctx: TaskContext,
        alert: Alert,
    ) -> Result<(), TaskError> {
        self.0.typed("open_status_page", &ctx, &alert).await
    }
    #[task(name = "ops.write_postmortem")]
    pub async fn write_postmortem(
        self: Arc<Self>,
        ctx: TaskContext,
        facts: (Alert, String, Option<Resolution>, Vec<String>, i64),
    ) -> Result<Postmortem, TaskError> {
        self.0.typed("write_postmortem", &ctx, &facts).await
    }
}

pub fn defaults(probe: &Probe) {
    probe.default_output("diagnose", |_| json!("diagnostics: ok"));
    probe.default_output(
        "lookup_owner",
        |_| json!({ "escalation": ["ana", "ben", "cy"] }),
    );
    probe.default_output("page", |_| json!(null));
    probe.default_output("open_status_page", |_| json!(null));
    probe.default_output("write_postmortem", |facts| {
        json!({ "duration_secs": facts[4], "notes": facts[3], "resolved": !facts[2].is_null() })
    });
}

#[workflow(namespace = "ops", name = "diagnostics", version = "1.0.0")]
pub struct Diagnostics;

#[workflow_methods]
impl Diagnostics {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, alert: Alert) -> WorkflowResult<String> {
        Ok(ctx.call(Ops::diagnose, &alert).await?)
    }
}

#[workflow(namespace = "ops", name = "incident", version = "1.0.0")]
pub struct Incident {
    acked_by: Option<String>,
    notes: Vec<String>,
}

#[workflow_methods]
impl Incident {
    #[init]
    fn new(_ctx: &WorkflowContextView) -> Self {
        Self {
            acked_by: None,
            notes: Vec::new(),
        }
    }

    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, alert: Alert) -> WorkflowResult<Postmortem> {
        let opened = ctx.now();
        let (diag, owner) = futures::try_join!(
            ctx.run(Diagnostics::run, &alert),
            ctx.call(Ops::lookup_owner, &alert.service),
        )?;

        let mut level = 0;
        while ctx.state(|s| s.acked_by.is_none()) {
            ctx.call(Ops::page, &(owner.escalation[level].clone(), alert.clone()))
                .await?;
            jackdaw::select! {
                _ = ctx.wait_condition(|s| s.acked_by.is_some()) => {},
                _ = ctx.sleep(15.minutes()) => { level = (level + 1).min(owner.escalation.len() - 1); },
            }
        }

        if alert.severity == Severity::Critical {
            ctx.call(Ops::open_status_page, &alert).await?;
        }

        let resolution = ctx
            .wait_for_event::<Resolution>("resolved")
            .timeout(7.days())
            .await;
        let ended = ctx.now();
        let notes = ctx.state(|s| s.notes.clone());
        Ok(ctx
            .call(
                Ops::write_postmortem,
                &(
                    alert,
                    diag,
                    resolution.ok(),
                    notes,
                    (ended - opened).num_seconds(),
                ),
            )
            .await?)
    }

    #[signal]
    fn ack(&mut self, _ctx: &mut SyncWorkflowContext<Self>, who: String) {
        self.acked_by = Some(who);
    }

    #[signal]
    fn note(&mut self, _ctx: &mut SyncWorkflowContext<Self>, text: String) {
        self.notes.push(text);
    }

    #[query]
    fn acked_by(&self, _ctx: &WorkflowContextView) -> Option<String> {
        self.acked_by.clone()
    }
}

pub fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture::of::<Diagnostics>("ops.diagnostics"),
        Fixture::of::<Incident>("ops.incident").input(alert("Major")),
    ]
}

pub fn by_code() -> Vec<(&'static str, &'static str)> {
    vec![("struct Incident {", "ops.incident")]
}

fn alert(severity: &str) -> Value {
    json!({ "service": "checkout", "severity": severity })
}

async fn start(world: &mut LibWorld, severity: &str) {
    world
        .start_fixture("it", super::fixture("ops.incident"), alert(severity))
        .await;
    world.quiesce().await;
}

async fn ack(world: &mut LibWorld, who: &str) {
    let handle = world.handle(world.last());
    handle
        .signal(Incident::ack, who.to_string())
        .await
        .expect("ack");
    world.quiesce().await;
}

#[when(regex = r#"^I execute "Incident"(?: for a "(\w+)" alert)?$"#)]
async fn execute_incident(world: &mut LibWorld, severity: String) {
    if severity.is_empty() {
        world
            .probe
            .script("diagnose", |s| s.delay = Some(Duration::from_millis(100)));
        world.probe.script("lookup_owner", |s| {
            s.delay = Some(Duration::from_millis(100))
        });
    }
    let severity = if severity.is_empty() {
        "Major".to_string()
    } else {
        severity
    };
    start(world, &severity).await;
}

#[when(regex = r#"^after (\d+ \w+) I signal "ack" with "(\w+)"$"#)]
async fn ack_after(world: &mut LibWorld, span: String, who: String) {
    world.pass(duration(&span)).await;
    ack(world, &who).await;
}

#[when(regex = r#"^I send the event "resolved"(?: after (\d+ \w+))?$"#)]
async fn send_resolved(world: &mut LibWorld, after: String) {
    if !after.is_empty() {
        let since_start = duration(&after);
        let started = world.runs[world.last()].started_virtual;
        let elapsed = (world.clock.now() - started).to_std().unwrap_or_default();
        world.pass(since_start.saturating_sub(elapsed)).await;
    }
    let handle = world.handle(world.last());
    handle
        .send_event(
            "resolved",
            Resolution {
                summary: "fixed".into(),
            },
        )
        .await
        .expect("send");
}

#[then(regex = r"^the postmortem has a duration of (\d+) hours$")]
async fn postmortem_duration(world: &mut LibWorld, hours: i64) {
    let last = world.last();
    assert_eq!(
        world.outcome(last).await.value()["duration_secs"],
        json!(hours * 3600)
    );
}

#[then(regex = r#"^the "Diagnostics" child and "LookupOwner" overlapped in time$"#)]
async fn diag_overlap(world: &mut LibWorld) {
    world.quiesce().await;
    assert!(world.probe.overlapped("diagnose", "lookup_owner"));
}

#[when(regex = r#"^I execute "Incident" with an escalation chain of (\d+) people$"#)]
async fn with_chain(world: &mut LibWorld, n: usize) {
    let people: Vec<String> = (1..=n).map(|i| format!("person-{i}")).collect();
    world.probe.script("lookup_owner", |s| {
        s.output = Some(json!({ "escalation": people }))
    });
    start(world, "Major").await;
}

#[when(regex = r#"^(\d+ \w+) pass without an "ack"$"#)]
async fn pass_without_ack(world: &mut LibWorld, span: String) {
    world.pass(duration(&span)).await;
}

fn paged(world: &LibWorld) -> Vec<Value> {
    world
        .probe
        .calls_of("page")
        .into_iter()
        .map(|c| c.input[0].clone())
        .collect()
}

#[then(regex = r#"^"Page" ran (\d+) times, once per person in order$"#)]
fn paged_in_order(world: &mut LibWorld, n: usize) {
    let expected: Vec<Value> = (1..=n).map(|i| json!(format!("person-{i}"))).collect();
    assert_eq!(paged(world), expected);
}

#[when(regex = r"^(\d+) more minutes pass$")]
async fn more_minutes(world: &mut LibWorld, minutes: u64) {
    world.pass(Duration::from_secs(minutes * 60)).await;
}

#[then("the last person is paged again")]
fn last_again(world: &mut LibWorld) {
    let pages = paged(world);
    assert_eq!(pages.len(), 4, "{pages:?}");
    assert_eq!(pages[3], pages[2]);
}

#[when(regex = r#"^I execute "Incident" for a "(\w+)" alert and "ack" it$"#)]
async fn execute_and_ack(world: &mut LibWorld, severity: String) {
    start(world, &severity).await;
    ack(world, "sam").await;
}

#[when(regex = r#"^I execute "Incident", "ack" it and signal "note" with "(.+)" and "(.+)"$"#)]
async fn ack_and_notes(world: &mut LibWorld, first: String, second: String) {
    start(world, "Major").await;
    ack(world, "sam").await;
    let handle = world.handle(world.last());
    for text in [first, second] {
        handle.signal(Incident::note, text).await.expect("note");
    }
}

#[then("the postmortem contains both notes in order")]
async fn notes_in_order(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(
        world.outcome(last).await.value()["notes"],
        json!(["rolled back deploy", "cache flushed"])
    );
}

#[when(regex = r#"^I execute "Incident" and "ack" it$"#)]
async fn execute_ack(world: &mut LibWorld) {
    start(world, "Major").await;
    ack(world, "sam").await;
}

#[when(regex = r#"^(\d+) days pass without "resolved"$"#)]
async fn days_without_resolved(world: &mut LibWorld, days: u64) {
    world.pass(Duration::from_secs(days * 86_400)).await;
}

#[then(regex = r#"^"WritePostmortem" ran with no resolution$"#)]
async fn no_resolution(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(world.outcome(last).await.value()["resolved"], json!(false));
    assert_eq!(
        world.probe.calls_of("write_postmortem")[0].input[2],
        json!(null)
    );
}

#[when(regex = r#"^I execute "Incident" and (\d+) minutes pass without an "ack"$"#)]
async fn execute_then_wait(world: &mut LibWorld, minutes: u64) {
    start(world, "Major").await;
    world.pass(Duration::from_secs(minutes * 60)).await;
}

#[then("the second person has been paged")]
fn second_paged(world: &mut LibWorld) {
    assert!(paged(world).contains(&json!("ben")), "{:?}", paged(world));
}

#[then("the first person was paged only once")]
fn first_once(world: &mut LibWorld) {
    assert_eq!(
        paged(world).iter().filter(|p| **p == json!("ana")).count(),
        1
    );
}

#[given(regex = r#"^a "TestEngine" with self-replay enabled$"#)]
fn self_replay(_world: &mut LibWorld) {}

#[when(regex = r#"^I run the "first responder" and "escalation" scenarios on it$"#)]
fn run_on_test_engine(world: &mut LibWorld) {
    let probe = Arc::clone(&world.probe);
    let started = Instant::now();
    let results = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .expect("a runtime");
        runtime.block_on(async move {
            let engine = jackdaw::testing::TestEngine::builder()
                .tasks(Ops(probe))
                .workflow::<Incident>()
                .build()
                .expect("builds");
            let mut results = Vec::new();
            // First responder: acknowledged straight away, resolved two hours later.
            let quick = engine
                .execute(
                    Incident::run,
                    serde_json::from_value(alert("Major")).unwrap(),
                )
                .await
                .unwrap();
            quick
                .signal(Incident::ack, "sam".to_string())
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_secs(7200)).await;
            quick
                .send_event(
                    "resolved",
                    Resolution {
                        summary: "fixed".into(),
                    },
                )
                .await
                .unwrap();
            results.push(quick.await.map(|_| ()).map_err(|e| e.to_string()));
            // Escalation: nobody acknowledges for 40 minutes.
            let slow = engine
                .execute(
                    Incident::run,
                    serde_json::from_value(alert("Critical")).unwrap(),
                )
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_secs(40 * 60)).await;
            slow.signal(Incident::ack, "cy".to_string()).await.unwrap();
            slow.send_event(
                "resolved",
                Resolution {
                    summary: "fixed".into(),
                },
            )
            .await
            .unwrap();
            results.push(slow.await.map(|_| ()).map_err(|e| e.to_string()));
            results
        })
    })
    .join()
    .expect("the test engine thread");
    world.notes.insert("results".into(), json!(results));
    world
        .notes
        .insert("wall".into(), json!(started.elapsed().as_secs_f64()));
}

#[then(regex = r#"^both complete without "Error::NonDeterminism"$"#)]
fn both_complete(world: &mut LibWorld) {
    assert_eq!(world.notes["results"], json!([{"Ok": null}, {"Ok": null}]));
}
