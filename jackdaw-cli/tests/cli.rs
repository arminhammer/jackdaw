//! BDD runner for `tests/features`: runs the `jackdaw` binary in a temporary directory and looks at what it prints.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use cucumber::gherkin::Step;
use cucumber::{World as _, given, then, when};
use serde_json::Value;
use std::process::{Command, Output};

#[derive(Debug, cucumber::World)]
#[world(init = Self::new)]
struct CliWorld {
    dir: tempfile::TempDir,
    output: Option<Output>,
}

impl CliWorld {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("a temporary directory"),
            output: None,
        }
    }

    fn output(&self) -> &Output {
        self.output.as_ref().expect("a command was run")
    }

    fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.output().stdout).into_owned()
    }

    fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.output().stderr).into_owned()
    }
}

#[given(regex = r#"^a workflow "([^"]+)":$"#)]
fn workflow(world: &mut CliWorld, name: String, step: &Step) {
    let text = step.docstring.as_ref().expect("a docstring");
    std::fs::write(world.dir.path().join(name), text).expect("the workflow is written");
}

#[when(regex = r#"^I run "jackdaw (.+)"$"#)]
fn run(world: &mut CliWorld, args: String) {
    // Output is piped, as it is for a program that reads the result.
    world.output = Some(
        Command::new(env!("CARGO_BIN_EXE_jackdaw"))
            .args(args.split_whitespace())
            .current_dir(world.dir.path())
            .output()
            .expect("the binary runs"),
    );
}

#[then("it succeeds")]
fn succeeds(world: &mut CliWorld) {
    assert!(
        world.output().status.success(),
        "it failed:\n{}",
        world.stderr()
    );
}

#[then("it fails")]
fn fails(world: &mut CliWorld) {
    assert!(!world.output().status.success(), "it succeeded");
}

#[then(regex = r#"^standard output is exactly "(.*)" as JSON$"#)]
fn stdout_json(world: &mut CliWorld, text: String) {
    let got: Value = serde_json::from_str(&world.stdout()).expect("standard output is JSON");
    assert_eq!(got, Value::String(text));
}

#[then("standard output is empty")]
fn stdout_empty(world: &mut CliWorld) {
    assert_eq!(world.stdout(), "");
}

#[then("standard error is empty")]
fn stderr_empty(world: &mut CliWorld) {
    assert_eq!(world.stderr(), "");
}

#[then(regex = r#"^standard output contains "(.*)"$"#)]
fn stdout_contains(world: &mut CliWorld, text: String) {
    let out = world.stdout();
    assert!(out.contains(&text), "`{text}` is not in:\n{out}");
}

#[then(regex = r#"^standard error contains "(.*)"$"#)]
fn stderr_contains(world: &mut CliWorld, text: String) {
    let text = text.replace("\\\"", "\"");
    let err = world.stderr();
    assert!(err.contains(&text), "`{text}` is not in:\n{err}");
}

#[then("standard error has the lines:")]
fn stderr_lines(world: &mut CliWorld, step: &Step) {
    let err = world.stderr();
    let lines: Vec<&str> = err.lines().map(str::trim).collect();
    let mut from = 0;
    for row in &step.table.as_ref().expect("a table").rows {
        let want = row[0].trim();
        let at = lines[from..]
            .iter()
            .position(|line| line.ends_with(want))
            .unwrap_or_else(|| panic!("no line `{want}` (in order) in:\n{err}"));
        from += at + 1;
    }
}

#[then(regex = r#"^every line of standard error is a JSON object with an "(\w+)" field$"#)]
fn stderr_json(world: &mut CliWorld, field: String) {
    let err = world.stderr();
    assert!(!err.trim().is_empty(), "standard error is empty");
    for line in err.lines() {
        let value: Value = serde_json::from_str(line).unwrap_or_else(|e| panic!("`{line}`: {e}"));
        assert!(value.get(&field).is_some(), "no `{field}` in {line}");
    }
}

fn main() {
    futures::executor::block_on(CliWorld::cucumber().fail_on_skipped().run("tests/features"));
}
