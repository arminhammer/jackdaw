//! # A workflow's input is its configuration
//!
//! A workflow takes one input, and that input should say everything a run needs: where to read from, how many
//! workers, which endpoint. Describe it once as a struct. jackdaw reads that struct from whichever sources the user of
//! the program chooses, so the program needs no argument parsing of its own.
//!
//! ```
//! use jackdaw::prelude::*;
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Serialize, Deserialize, Config)]
//! struct Backup {
//!     /// Where to read from
//!     source: String,
//!     /// How many copies run at once
//!     #[serde(default = "four")]
//!     #[config(short = 'w')]
//!     workers: u8,
//!     /// Tags to apply, as often as needed
//!     #[serde(default)]
//!     tags: Vec<String>,
//!     /// Print what would be copied
//!     #[serde(default)]
//!     dry_run: bool,
//!     /// Limits are nested, so they come from files and the environment, not from flags
//!     #[serde(default)]
//!     #[config(skip)]
//!     limits: Limits,
//! }
//!
//! #[derive(Default, Serialize, Deserialize)]
//! struct Limits {
//!     retries: u8,
//! }
//!
//! fn four() -> u8 { 4 }
//! ```
//!
//! The struct is plain `serde`: defaults are `#[serde(default)]`, a required field has none, and any type that
//! `serde` can read works. `#[derive(Config)]` adds the flags.
//!
//! ## Sources
//!
//! A value is looked for in these places, and a later one wins over an earlier one, setting by setting:
//!
//! 1. `serde`'s defaults;
//! 2. files: `.toml`, `.json`, `.yaml` or `.yml`, chosen by their extension (`--config backup.toml`; repeat the flag to
//!    layer files);
//! 3. environment variables: the workflow's name in capitals, then `_`, then the setting (`BACKUP_WORKERS=8`), with
//!    `__` going into a nested setting (`BACKUP_LIMITS__RETRIES=5`). Values are read as JSON scalars, arrays and
//!    objects (`8`, `true`, `["a","b"]`) or as text;
//! 4. flags.
//!
//! The first three are [`config::Layers`](crate::config::Layers), which a program can use on its own, with any serde
//! type, and which tests can feed with `env_from` instead of the process's environment:
//!
//! ```
//! use jackdaw::config::Layers;
//! # #[derive(serde::Deserialize)] struct Backup { source: String, #[serde(default)] workers: u8 }
//! # fn main() -> Result<(), jackdaw::config::Error> {
//! let backup: Backup = Layers::new()
//!     .file_if_exists("backup.toml")                      // skipped if it is not there
//!     .env_from("BACKUP_", [("BACKUP_SOURCE", "/data")])  // `.env("BACKUP_")` reads the process's
//!     .load()?;
//! assert_eq!(backup.source, "/data");
//! # Ok(()) }
//! ```
//!
//! A missing or mistyped setting is an error that names it (`missing field `source``), and a file that does not exist
//! (when it was asked for) or has an unknown extension is one that names the file.
//!
//! ## Flags
//!
//! `#[derive(Config)]` makes one flag per field, named after it in kebab case (`dry_run` is `--dry-run`), with the
//! field's doc comment as its help.
//!
//! | The field | The flag |
//! | --- | --- |
//! | `String`, numbers, `PathBuf`, anything `FromStr + Serialize` | `--name value` |
//! | `Option<T>` | as `T`, and leaving it out leaves it unset |
//! | `Vec<T>` | `--name a --name b` |
//! | `bool` | `--name`, or `--name=false` to override a file that says true |
//! | marked `#[config(skip)]` | none: it still comes from files and the environment. Use it for nested structs |
//!
//! `#[config(short = 'w')]` adds a one-letter form and `#[config(long = "workers")]` renames the flag.
//! `#[serde(rename = "..")]` is followed. A flag the user leaves out says nothing, so it never overrides a file or the
//! environment. `rename_all` is not followed; give the fields `#[serde(rename)]`, or `#[config(long)]`, instead.
//!
//! ## Program
//!
//! With the `cli` feature, [`runner::Program`](crate::runner::Program) is `main` for a workflow:
//!
//! ```no_run
//! # use jackdaw::prelude::*;
//! # use jackdaw::runner::Program;
//! # #[derive(serde::Serialize, serde::Deserialize, Config)] struct Backup { source: String }
//! # #[workflow(namespace = "docs", name = "backup", version = "1")] struct BackupWorkflow;
//! # #[workflow_methods] impl BackupWorkflow { #[run] async fn run(_: &mut WorkflowContext<Self>, b: Backup) -> WorkflowResult<String> { Ok(b.source) } }
//! #[tokio::main]
//! async fn main() -> Result<std::process::ExitCode, Error> {
//!     let engine = Engine::builder().workflow::<BackupWorkflow>().build()?;
//!     let program = Program::<BackupWorkflow>::new(engine).config_file("backup.toml"); // read if it is there
//!     Ok(program.main().await)
//! }
//! ```
//!
//! It reads the input as above, runs the workflow with a [`Runner`](crate::runner::Runner), and prints the output as
//! JSON. Its own flags are `--config`/`-c`, `--run-id`, and `--quiet`, `--json`, `--plain` and `--restart` (see
//! [the runner chapter](crate::guide::runner)); a field of yours cannot use those names. The exit code is 0 for
//! success and help, 1 for a failed run, and 2 for a command line or an input that could not be read.
//! `Program::run(args, vars)` does the same for arguments and variables you give it, and returns the output instead of
//! printing it, which is how to test a program.
//!
//! A run without `--run-id` is a new run each time. With `--run-id` it is resumable, and the input is stored with it:
//! running the same id with other input is refused, unless `--restart` is given. That is how a configuration that
//! changed under a half-finished run is noticed instead of silently ignored.
//!
//! ## What belongs in the input
//!
//! Everything the run needs to decide what to do, and, as far as you can, everything it will touch: the paths, the
//! endpoints, the limits. A workflow that reads the environment itself (`std::env::var`) hides an input from its
//! journal and from replay; put it in the config instead, where it is recorded and compared.
