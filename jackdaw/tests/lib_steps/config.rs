//! operations/config.feature: `jackdaw::config`, a workflow's input from files, the environment and arguments.
use super::world::LibWorld;
use cucumber::gherkin::Step;
use cucumber::{given, then, when};
use jackdaw::config::{Error, Layers};
use jackdaw::prelude::*;
use jackdaw::runner::{Program, ProgramError};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Deserialize, Serialize, Config)]
pub struct Settings {
    /// Where to read from
    source: String,
    /// How many copies run at once
    #[serde(default = "four")]
    #[config(short = 'w')]
    workers: u8,
    #[serde(default)]
    verbose: bool,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    #[config(skip)]
    limits: Limits,
}

#[workflow(namespace = "config", name = "backup", version = "1")]
pub struct Backup;

#[workflow_methods]
impl Backup {
    #[run]
    async fn run(_: &mut WorkflowContext<Self>, s: Settings) -> WorkflowResult<String> {
        Ok(format!(
            "{}:{}:{}:{}:{}",
            s.source,
            s.workers,
            s.verbose,
            s.tags.join(","),
            s.limits.retries
        ))
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct Limits {
    #[serde(default)]
    retries: u8,
}

fn four() -> u8 {
    4
}

#[derive(Default)]
pub struct State {
    environment: Vec<(String, String)>,
    arguments: Map<String, Value>,
    loaded: Option<Result<Settings, Error>>,
    engine: Option<Engine>,
    outcome: Option<Result<String, ProgramError>>,
}

fn rows(step: &Step) -> Vec<(String, String)> {
    step.table()
        .expect("a table")
        .rows
        .iter()
        .skip(1)
        .map(|r| (r[0].clone(), r[1].clone()))
        .collect()
}

#[given(regex = r#"^a settings file "([^"]+)" containing:$"#)]
async fn settings_file(world: &mut LibWorld, name: String, step: &Step) {
    let text = step.docstring().expect("a docstring").trim().to_string();
    std::fs::write(world.dir.path().join(name), text.replace("\\n", "\n")).expect("write");
}

#[given("the environment has:")]
async fn environment(world: &mut LibWorld, step: &Step) {
    world.config.environment = rows(step);
}

#[given("the arguments are:")]
async fn arguments(world: &mut LibWorld, step: &Step) {
    for (name, value) in rows(step) {
        world.config.arguments.insert(name, Value::String(value));
    }
}

fn path(world: &LibWorld, name: &str) -> std::path::PathBuf {
    world.dir.path().join(name)
}

fn finish(world: &mut LibWorld, layers: Layers) {
    world.config.loaded = Some(layers.load::<Settings>());
}

#[when(regex = r#"^the settings are loaded from "([^"]+)"$"#)]
async fn from_file(world: &mut LibWorld, name: String) {
    let layers = Layers::new().file(path(world, &name));
    finish(world, layers);
}

#[when(regex = r#"^the settings are loaded from "([^"]+)" then "([^"]+)"$"#)]
async fn from_two_files(world: &mut LibWorld, first: String, second: String) {
    let layers = Layers::new()
        .file(path(world, &first))
        .file(path(world, &second));
    finish(world, layers);
}

#[when("the settings are loaded from no source")]
async fn from_nothing(world: &mut LibWorld) {
    finish(world, Layers::new());
}

#[when(regex = r#"^the settings are loaded from "([^"]+)" and the "([^"]+)" environment$"#)]
async fn from_file_and_env(world: &mut LibWorld, name: String, prefix: String) {
    let layers = Layers::new()
        .file(path(world, &name))
        .env_from(&prefix, world.config.environment.clone());
    finish(world, layers);
}

#[when(regex = r#"^the settings are loaded from the "([^"]+)" environment$"#)]
async fn from_env(world: &mut LibWorld, prefix: String) {
    let layers = Layers::new().env_from(&prefix, world.config.environment.clone());
    finish(world, layers);
}

#[when(
    regex = r#"^the settings are loaded from the optional "([^"]+)" and the "([^"]+)" environment$"#
)]
async fn from_optional(world: &mut LibWorld, name: String, prefix: String) {
    let layers = Layers::new()
        .file_if_exists(path(world, &name))
        .env_from(&prefix, world.config.environment.clone());
    finish(world, layers);
}

#[when(
    regex = r#"^the settings are loaded from "([^"]+)", the "([^"]+)" environment and the arguments$"#
)]
async fn from_all(world: &mut LibWorld, name: String, prefix: String) {
    let layers = Layers::new()
        .file(path(world, &name))
        .env_from(&prefix, world.config.environment.clone())
        .args(&world.config.arguments);
    finish(world, layers);
}

fn loaded(world: &LibWorld) -> &Settings {
    match world.config.loaded.as_ref().expect("settings were loaded") {
        Ok(s) => s,
        Err(e) => panic!("loading failed: {e}"),
    }
}

#[then(regex = r#"^the settings are source "([^"]*)", workers (\d+), verbose (true|false)$"#)]
async fn settings_are(world: &mut LibWorld, source: String, workers: u8, verbose: bool) {
    let s = loaded(world);
    assert_eq!(
        (s.source.as_str(), s.workers, s.verbose),
        (source.as_str(), workers, verbose)
    );
}

#[then(regex = r"^the retries limit is (\d+)$")]
async fn retries(world: &mut LibWorld, expected: u8) {
    assert_eq!(loaded(world).limits.retries, expected);
}

#[then(regex = r#"^loading fails and the message mentions "([^"]+)"$"#)]
async fn fails(world: &mut LibWorld, text: String) {
    match world.config.loaded.as_ref().expect("settings were loaded") {
        Ok(s) => panic!("loading succeeded: {s:?}"),
        Err(e) => assert!(e.to_string().contains(&text), "{e}"),
    }
}

#[given("a backup program")]
async fn backup_program(world: &mut LibWorld) {
    world.config.engine = Some(
        Engine::builder()
            .workflow::<Backup>()
            .build()
            .expect("an engine"),
    );
}

#[when(regex = r#"^the backup program is run with "([^"]*)"$"#)]
async fn program_runs(world: &mut LibWorld, args: String) {
    let args = args.replace("{dir}", &world.dir.path().display().to_string());
    let program = Program::<Backup>::new(world.config.engine.clone().expect("a program"));
    let result = program
        .run(
            std::iter::once("backup".to_string()).chain(args.split_whitespace().map(String::from)),
            world.config.environment.clone(),
        )
        .await;
    world.config.outcome = Some(result);
}

fn failure(world: &LibWorld) -> &ProgramError {
    match world.config.outcome.as_ref().expect("the program ran") {
        Ok(out) => panic!("the program printed {out}"),
        Err(e) => e,
    }
}

#[then(regex = r#"^the program prints "([^"]*)"$"#)]
async fn program_prints(world: &mut LibWorld, expected: String) {
    match world.config.outcome.as_ref().expect("the program ran") {
        Ok(out) => assert_eq!(out, &expected),
        Err(e) => panic!("the program failed: {e}"),
    }
}

#[then(regex = r#"^the program fails with usage error mentioning "([^"]*)"$"#)]
async fn usage_error(world: &mut LibWorld, text: String) {
    let e = failure(world);
    assert!(
        matches!(e, ProgramError::Usage { code, .. } if *code == 2),
        "{e:?}"
    );
    assert!(e.to_string().contains(&text), "{e}");
}

#[then(regex = r#"^the program fails with a configuration error mentioning "([^"]*)"$"#)]
async fn config_error(world: &mut LibWorld, text: String) {
    let e = failure(world);
    assert!(matches!(e, ProgramError::Config(_)), "{e:?}");
    assert!(e.to_string().contains(&text), "{e}");
}

#[then(regex = r#"^the program fails with a run error mentioning "([^"]*)"$"#)]
async fn run_error(world: &mut LibWorld, text: String) {
    let e = failure(world);
    assert!(matches!(e, ProgramError::Run(_)), "{e:?}");
    assert!(e.to_string().contains(&text), "{e}");
}

#[then("the program shows usage mentioning:")]
async fn usage_mentions(world: &mut LibWorld, step: &Step) {
    let e = failure(world);
    assert!(matches!(e, ProgramError::Usage { code: 0, .. }), "{e:?}");
    for row in step.table().expect("a table").rows.iter() {
        assert!(e.to_string().contains(row[0].trim()), "{e}");
    }
}

#[then(regex = r#"^the usage does not mention "([^"]*)"$"#)]
async fn usage_omits(world: &mut LibWorld, text: String) {
    assert!(!failure(world).to_string().contains(&text));
}
