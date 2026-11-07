use super::{Runner, RunnerArgs};
use crate::api::{Engine, WorkflowDef, WorkflowRef};
use crate::config::{Config, Flag, FlagKind, Layers};
use clap::{Arg, ArgAction, ArgMatches, Args, Command, FromArgMatches};
use serde_json::{Map, Value};
use std::ffi::OsString;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::process::ExitCode;

/// A workflow as a program: its input comes from files, the environment and flags, and the run is reported as it goes.
///
/// ```no_run
/// use jackdaw::prelude::*;
/// use jackdaw::runner::Program;
///
/// #[derive(serde::Serialize, serde::Deserialize, Config)]
/// struct Greet {
///     /// Who to greet
///     name: String,
/// }
///
/// #[workflow(namespace = "docs", name = "greet", version = "1")]
/// struct Greeter;
///
/// #[workflow_methods]
/// impl Greeter {
///     #[run]
///     async fn run(_: &mut WorkflowContext<Self>, input: Greet) -> WorkflowResult<String> {
///         Ok(format!("hello, {}!", input.name))
///     }
/// }
///
/// #[tokio::main]
/// async fn main() -> Result<std::process::ExitCode, Error> {
///     let engine = Engine::builder().workflow::<Greeter>().build()?;
///     Ok(Program::<Greeter>::new(engine).main().await)
/// }
/// ```
///
/// The input is read from these, each overriding the ones before:
/// 1. `serde`'s defaults on the input type;
/// 2. the files given to [`config_file`](Self::config_file) that exist, then each `--config <file>` (`.toml`, `.json`,
///    `.yaml`);
/// 3. environment variables that start with the [prefix](Self::env_prefix) (by default the workflow's name in capitals,
///    then `_`), `__` going into a nested setting;
/// 4. the flags `#[derive(Config)]` makes, one per field.
///
/// The program has these flags of its own as well: `--config`/`-c`, `--run-id`, and the `--quiet`, `--json`, `--plain` and
/// `--restart` of `RunnerArgs`. Without `--run-id`, every run is a new one; with it, the run is resumable and a second
/// call with other input is refused unless it has `--restart`. The output is written to standard output as JSON.
pub struct Program<W> {
    engine: Engine,
    prefix: String,
    files: Vec<PathBuf>,
    workflow: PhantomData<fn() -> W>,
}

/// Why a [`Program`] did not give a result.
#[derive(Debug)]
#[non_exhaustive]
pub enum ProgramError {
    /// The command line was not understood, or it asked for help. `code` is the exit code: 0 for help.
    Usage {
        /// What to show.
        message: String,
        /// The exit code.
        code: u8,
    },
    /// The input could not be read from its sources.
    Config(crate::config::Error),
    /// The run failed.
    Run(super::Error),
}

impl std::fmt::Display for ProgramError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usage { message, .. } => f.write_str(message),
            Self::Config(e) => write!(f, "invalid input: {e}"),
            Self::Run(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ProgramError {}

impl ProgramError {
    /// The exit code: 0 for help, 2 for input that could not be read, 1 for a run that failed.
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Usage { code, .. } => *code,
            Self::Config(_) => 2,
            Self::Run(_) => 1,
        }
    }
}

impl<W> Program<W>
where
    W: WorkflowDef,
    W::In: Config,
{
    /// A program that runs `W` on `engine`, which has `W` and its tasks registered.
    #[must_use]
    pub fn new(engine: Engine) -> Self {
        let prefix: String = W::NAME
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect();
        Self {
            engine,
            prefix: format!("{prefix}_"),
            files: Vec::new(),
            workflow: PhantomData,
        }
    }

    /// The prefix of the environment variables that give the input.
    #[must_use]
    pub fn env_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }

    /// A file read if it exists, before any `--config` file: `~/.config/app.toml`, `./app.toml`.
    #[must_use]
    pub fn config_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.files.push(path.into());
        self
    }

    fn command() -> Command {
        let command = Command::new(W::NAME)
            .about(format!("{}/{} {}", W::NAMESPACE, W::NAME, W::VERSION))
            .arg(
                Arg::new("jackdaw:config")
                    .long("config")
                    .short('c')
                    .value_name("FILE")
                    .value_parser(clap::value_parser!(PathBuf))
                    .action(ArgAction::Append)
                    .help("Read the input from this file (.toml, .json, .yaml); repeat to layer files"),
            )
            .arg(
                Arg::new("jackdaw:id")
                    .long("run-id")
                    .value_name("ID")
                    .help("Name the run, so that it can be resumed and not repeated"),
            );
        let command = RunnerArgs::augment_args(command);
        W::In::flags().iter().fold(command, |c, f| c.arg(arg(f)))
    }

    /// Reads the input from `args` (the first is the program's name) and `vars`, runs the workflow and returns its output.
    ///
    /// # Errors
    /// [`ProgramError`]: the command line, the input, or the run.
    pub async fn run<I, T, K, V>(
        self,
        args: I,
        vars: impl IntoIterator<Item = (K, V)>,
    ) -> Result<W::Out, ProgramError>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let matches =
            Self::command()
                .try_get_matches_from(args)
                .map_err(|e| ProgramError::Usage {
                    message: e.render().to_string(),
                    code: u8::try_from(e.exit_code()).unwrap_or(2),
                })?;
        let runner_args =
            RunnerArgs::from_arg_matches(&matches).map_err(|e| ProgramError::Usage {
                message: e.to_string(),
                code: 2,
            })?;
        let mut layers = Layers::new();
        for file in &self.files {
            layers = layers.file_if_exists(file);
        }
        for file in matches
            .get_many::<PathBuf>("jackdaw:config")
            .into_iter()
            .flatten()
        {
            layers = layers.file(file);
        }
        let input: W::In = layers
            .env_from(&self.prefix, vars)
            .args(overrides(&W::In::flags(), &matches))
            .load()
            .map_err(ProgramError::Config)?;
        let id = matches
            .get_one::<String>("jackdaw:id")
            .cloned()
            .unwrap_or_else(|| format!("{}/{}/{}", W::NAMESPACE, W::NAME, uuid::Uuid::new_v4()));
        Runner::new(&self.engine)
            .reporter(runner_args.reporter())
            .run(
                WorkflowRef::<W, W::In, W::Out>::new(),
                input,
                runner_args.options(id),
            )
            .await
            .map_err(ProgramError::Run)
    }

    /// Runs the program with the process's arguments and environment, prints the output as JSON and gives the exit code.
    pub async fn main(self) -> ExitCode {
        match self.run(std::env::args_os(), std::env::vars()).await {
            Ok(output) => match serde_json::to_string_pretty(&output) {
                Ok(text) if text != "null" => {
                    println!("{text}");
                    ExitCode::SUCCESS
                }
                Ok(_) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("the output could not be written: {e}");
                    ExitCode::FAILURE
                }
            },
            Err(e) => {
                if e.exit_code() == 0 {
                    println!("{e}");
                } else {
                    eprintln!("{e}");
                }
                ExitCode::from(e.exit_code())
            }
        }
    }
}

fn arg(flag: &Flag) -> Arg {
    let parse = flag.parse;
    let arg = Arg::new(format!("cfg:{}", flag.key))
        .long(flag.long)
        .help(flag.help)
        .value_name(flag.long.to_ascii_uppercase())
        .value_parser(move |text: &str| parse(text));
    let arg = match flag.short {
        Some(short) => arg.short(short),
        None => arg,
    };
    match flag.kind {
        FlagKind::Value => arg.num_args(1),
        FlagKind::List => arg.action(ArgAction::Append),
        FlagKind::Switch => arg
            .num_args(0..=1)
            .require_equals(true)
            .default_missing_value("true"),
    }
}

/// What the flags the user gave say; the flags left out say nothing.
fn overrides(flags: &[Flag], matches: &ArgMatches) -> Map<String, Value> {
    let mut given = Map::new();
    for flag in flags {
        let id = format!("cfg:{}", flag.key);
        let value = match flag.kind {
            FlagKind::List => matches
                .get_many::<Value>(&id)
                .map(|values| Value::Array(values.cloned().collect())),
            FlagKind::Value | FlagKind::Switch => matches.get_one::<Value>(&id).cloned(),
        };
        if let Some(value) = value {
            given.insert(flag.key.to_string(), value);
        }
    }
    given
}
