//! Runs a workflow as a program: starts, resumes or restarts it, and reports its progress.
//!
//! A [`Runner`] wraps an [`Engine`]. [`Runner::run`] looks at what the engine's store knows about the instance and does
//! the right thing: it starts a new run, resumes one a crash stopped, hands back the result of one that finished, and
//! refuses one that was started with different input or ended in failure (unless asked to [restart](RunOptions::restart)).
//! What the run shows is up to its [`Reporter`]:
//!
//! | reporter | shows |
//! | --- | --- |
//! | [`Headless`] | nothing |
//! | [`Lines`] | one line per step, for logs and CI |
//! | [`Json`] | one JSON object per event, for machines |
//! | `Terminal` (feature `terminal`) | a progress bar and a summary |
//!
//! [`Env::detect`] picks one from where the program runs (a terminal, a pipe, a flag), and `RunnerArgs` (feature `cli`)
//! gives a `clap` program the flags for it.
//!
//! ```no_run
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! use jackdaw::prelude::*;
//! use jackdaw::runner::{Env, RunOptions, Runner, reporter};
//!
//! # #[workflow(namespace = "docs", name = "pipeline", version = "1")] struct Pipeline;
//! # #[workflow_methods] impl Pipeline { #[run] async fn run(_: &mut WorkflowContext<Self>, _: String) -> WorkflowResult<String> { Ok(String::new()) } }
//! let engine = Engine::builder().workflow::<Pipeline>().build()?;
//! let mut runner = Runner::new(&engine).reporter(reporter(Env::detect().kind()));
//! let output = runner
//!     .run(Pipeline::run, "input".to_string(), RunOptions::new("pipeline/today"))
//!     .await?;
//! # let _ = output; Ok(()) }
//! ```

#[cfg(all(feature = "cli", not(target_family = "wasm")))]
mod args;
mod lines;
#[cfg(all(feature = "cli", not(target_family = "wasm")))]
mod program;
#[cfg(all(feature = "terminal", not(target_family = "wasm")))]
mod terminal;

#[cfg(all(feature = "cli", not(target_family = "wasm")))]
pub use args::RunnerArgs;
pub use lines::{Json, Lines};
#[cfg(all(feature = "cli", not(target_family = "wasm")))]
pub use program::{Program, ProgramError};
#[cfg(all(feature = "terminal", not(target_family = "wasm")))]
pub use terminal::Terminal;

use crate::api::{Engine, Event, ExecuteOptions, Runnable, Status};
use serde_json::Value;
use std::future::IntoFuture;
use std::io::IsTerminal;
use std::time::Duration;

/// What a [`Runner`] tells its [`Reporter`].
#[derive(Debug)]
#[non_exhaustive]
pub enum Report<'a> {
    /// A new run is starting.
    Starting {
        /// The instance.
        id: &'a str,
    },
    /// A run that was stopped is carrying on from its journal.
    Resuming {
        /// The instance.
        id: &'a str,
    },
    /// The earlier run is discarded and a new one starts.
    Restarting {
        /// The instance.
        id: &'a str,
    },
    /// The run had finished already; its stored result follows and no events do.
    Finished {
        /// The instance.
        id: &'a str,
    },
    /// An event of the run. Steps replayed after a resume arrive with `replayed` set.
    Event(&'a Event),
    /// The run ended, one way or another: the last report.
    Done,
}

/// Shows a run's progress.
pub trait Reporter: Send {
    /// Handles one report. Called from the task that awaits the run, so it should not block for long.
    fn report(&mut self, report: Report<'_>);
}

impl Reporter for Box<dyn Reporter> {
    fn report(&mut self, report: Report<'_>) {
        (**self).report(report);
    }
}

/// Shows nothing: for services, tests, and programs that report in their own way.
#[derive(Debug, Default, Clone, Copy)]
pub struct Headless;

impl Reporter for Headless {
    fn report(&mut self, _report: Report<'_>) {}
}

/// Why [`Runner::run`] did not give a result.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The instance exists and was started with other input. Running it again would not do what was asked.
    InputChanged {
        /// The instance.
        id: String,
        /// The input it was started with.
        stored: Value,
        /// The input given now.
        given: Value,
    },
    /// The instance ended without a result.
    Ended {
        /// The instance.
        id: String,
        /// How it ended.
        status: Status,
    },
    /// The engine or the workflow failed.
    Run(crate::api::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InputChanged { id, stored, given } => write!(
                f,
                "`{id}` was started with input {stored}, not {given}; run it with the same input, or restart it to start over"
            ),
            Self::Ended { id, status } => {
                write!(f, "`{id}` ended as {status:?}; restart it to start over")
            }
            Self::Run(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Run(error) => Some(error),
            Self::InputChanged { .. } | Self::Ended { .. } => None,
        }
    }
}

impl From<crate::api::Error> for Error {
    fn from(error: crate::api::Error) -> Self {
        Self::Run(error)
    }
}

/// Which run [`Runner::run`] is about.
#[derive(Debug, Clone)]
pub struct RunOptions {
    id: String,
    restart: bool,
    timeout: Option<Duration>,
}

impl RunOptions {
    /// The run called `id`. The id is what makes a run resumable: the same id finds the same run.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            restart: false,
            timeout: None,
        }
    }

    /// Ends the run with a timeout error if it is still going this long after it started. Stored with the instance, so
    /// the deadline holds across restarts. For Rust workflows: a YAML workflow's deadline is its own `timeout`, and
    /// running one with this option fails.
    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Discards an earlier run with this id, whatever state it is in, and starts over.
    #[must_use]
    pub fn restart(mut self, restart: bool) -> Self {
        self.restart = restart;
        self
    }
}

/// Starts, resumes or restarts runs on an engine and shows their progress. See the [module](self) documentation.
pub struct Runner<R = Headless> {
    engine: Engine,
    reporter: R,
}

impl Runner<Headless> {
    /// A runner that shows nothing; give it a reporter with [`reporter`](Runner::reporter).
    #[must_use]
    pub fn new(engine: &Engine) -> Self {
        Self {
            engine: engine.clone(),
            reporter: Headless,
        }
    }
}

/// How long the runner waits for the last events after the result arrives.
const LAST_EVENTS: Duration = Duration::from_millis(250);

impl<R: Reporter> Runner<R> {
    /// The same runner, showing its runs with `reporter`.
    #[must_use]
    pub fn reporter<N: Reporter>(self, reporter: N) -> Runner<N> {
        Runner {
            engine: self.engine,
            reporter,
        }
    }

    /// The engine the runner runs on.
    #[must_use]
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Where the run called `id` is, or `None` if this engine's store has never seen it.
    ///
    /// # Errors
    /// If the store cannot be read.
    pub async fn status(&self, id: &str) -> Result<Option<Status>, crate::api::Error> {
        match self.engine.status(id).await {
            Ok(status) => Ok(Some(status)),
            Err(crate::api::Error::Engine(message)) if message.contains("was not found") => {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    /// Runs `runnable` as the instance `options` names, and returns its output.
    ///
    /// - No such instance: it starts, and the reporter gets [`Report::Starting`].
    /// - It is running (its engine stopped, or a previous process died): it carries on from its journal
    ///   ([`Report::Resuming`]), and the steps already done arrive as replayed events.
    /// - It finished: its stored output is returned without running anything ([`Report::Finished`]).
    /// - It ended in failure or was cancelled: [`Error::Ended`], unless the options ask for a restart.
    /// - It was started with other input: [`Error::InputChanged`], unless the options ask for a restart.
    ///
    /// # Errors
    /// As above, and [`Error::Run`] if the engine or the workflow fails.
    // `Status` and `Event` can grow: what a reporter does not know it leaves alone.
    #[allow(clippy::wildcard_enum_match_arm)]
    pub async fn run<T: Runnable>(
        &mut self,
        runnable: T,
        input: T::In,
        options: RunOptions,
    ) -> Result<T::Out, Error> {
        let given = serde_json::to_value(&input).map_err(|e| {
            Error::Run(crate::api::Error::invalid(format!(
                "the input could not be serialized: {e}"
            )))
        })?;
        let RunOptions {
            id,
            restart,
            timeout,
        } = options;
        let known = self.status(&id).await?;
        let mut finished = false;
        match known {
            Some(_) if restart => {
                self.reporter.report(Report::Restarting { id: &id });
                self.engine.purge(&id).await?;
            }
            Some(status) => {
                let stored = self.engine.describe(&id).await?.input;
                if let Some(stored) = stored.filter(|stored| *stored != given) {
                    return Err(Error::InputChanged { id, stored, given });
                }
                match status {
                    Status::Running => self.reporter.report(Report::Resuming { id: &id }),
                    Status::Completed => {
                        finished = true;
                        self.reporter.report(Report::Finished { id: &id });
                    }
                    other => return Err(Error::Ended { id, status: other }),
                }
            }
            None => self.reporter.report(Report::Starting { id: &id }),
        }
        let mut start = ExecuteOptions::new().id(&id);
        if let Some(timeout) = timeout {
            start = start.timeout(timeout);
        }
        let handle = self.engine.execute_with(runnable, input, start).await?;
        if finished {
            let result = handle.await;
            self.reporter.report(Report::Done);
            return Ok(result?);
        }
        let mut events = handle.events();
        let mut result = std::pin::pin!(handle.into_future());
        let mut ended = false;
        let result = loop {
            tokio::select! {
                biased;
                result = &mut result => break result,
                event = events.next() => match event {
                    Some(event) => {
                        ended |= event.is_terminal();
                        self.reporter.report(Report::Event(&event));
                    }
                    None => break result.await,
                },
            }
        };
        while !ended {
            match tokio::time::timeout(LAST_EVENTS, events.next()).await {
                Ok(Some(event)) => {
                    ended = event.is_terminal();
                    self.reporter.report(Report::Event(&event));
                }
                Ok(None) | Err(_) => break,
            }
        }
        self.reporter.report(Report::Done);
        Ok(result?)
    }
}

/// How a program shows a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Kind {
    /// [`Headless`].
    Headless,
    /// [`Lines`].
    Lines,
    /// [`Json`].
    Json,
    /// `Terminal`; without the `terminal` feature, [`reporter`] gives [`Lines`] instead.
    Terminal,
}

/// What decides how a program shows a run.
#[derive(Debug, Clone, Copy, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct Env {
    /// Progress goes to a terminal that can redraw it.
    pub terminal: bool,
    /// That terminal is `TERM=dumb`, which cannot.
    pub dumb: bool,
    /// The user asked for nothing to be shown.
    pub quiet: bool,
    /// The user asked for JSON lines.
    pub json: bool,
}

impl Env {
    /// Looks at standard error (where progress goes) and `TERM`. `quiet` and `json` are left off: they are the user's
    /// to ask for.
    #[must_use]
    pub fn detect() -> Self {
        Self {
            terminal: std::io::stderr().is_terminal(),
            dumb: std::env::var("TERM").is_ok_and(|term| term == "dumb"),
            quiet: false,
            json: false,
        }
    }

    /// The reporter that fits: nothing if quiet, JSON lines if asked for, a progress bar on a terminal that can draw
    /// one, and plain lines everywhere else.
    #[must_use]
    pub fn kind(&self) -> Kind {
        if self.quiet {
            Kind::Headless
        } else if self.json {
            Kind::Json
        } else if self.terminal && !self.dumb {
            Kind::Terminal
        } else {
            Kind::Lines
        }
    }
}

/// The reporter for `kind`, writing to standard error.
#[must_use]
pub fn reporter(kind: Kind) -> Box<dyn Reporter> {
    match kind {
        Kind::Headless => Box::new(Headless),
        Kind::Json => Box::new(Json::stderr()),
        #[cfg(feature = "terminal")]
        Kind::Terminal => Box::new(Terminal::new()),
        #[cfg(not(feature = "terminal"))]
        Kind::Terminal => Box::new(Lines::stderr()),
        Kind::Lines => Box::new(Lines::stderr()),
    }
}
