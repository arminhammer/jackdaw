use super::{Env, Kind, Reporter, RunOptions};
use clap::{Args, FromArgMatches};

/// The flags of a program that runs a workflow with a [`Runner`](super::Runner), for `clap`'s derive API:
///
/// ```no_run
/// # use clap::Parser;
/// use jackdaw::runner::RunnerArgs;
///
/// #[derive(Parser)]
/// struct Cli {
///     #[command(flatten)]
///     runner: RunnerArgs,
/// }
/// ```
#[derive(Debug, Clone, Default, Args)]
#[allow(clippy::struct_excessive_bools)]
pub struct RunnerArgs {
    /// Show nothing while the run is going.
    #[arg(short, long)]
    pub quiet: bool,
    /// Write one JSON object per event instead of progress.
    #[arg(long, conflicts_with = "quiet")]
    pub json: bool,
    /// Write plain lines, not a progress bar, even on a terminal.
    #[arg(long, conflicts_with_all = ["quiet", "json"])]
    pub plain: bool,
    /// Discard an earlier run of the same name and start over.
    #[arg(long)]
    pub restart: bool,
}

impl RunnerArgs {
    /// Parses `args` (the first is the program's name) the way a program with only these flags would.
    ///
    /// # Errors
    /// The message clap gives for arguments it does not accept.
    pub fn try_parse_from<I, T>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        let command = Self::augment_args(clap::Command::new("program"));
        let matches = command
            .try_get_matches_from(args)
            .map_err(|e| e.to_string())?;
        Self::from_arg_matches(&matches).map_err(|e| e.to_string())
    }

    /// How to show the run, given where the program runs and these flags.
    #[must_use]
    pub fn kind(&self, env: Env) -> Kind {
        Env {
            terminal: env.terminal && !self.plain,
            quiet: env.quiet || self.quiet,
            json: env.json || self.json,
            ..env
        }
        .kind()
    }

    /// The reporter for [`Env::detect`] and these flags.
    #[must_use]
    pub fn reporter(&self) -> Box<dyn Reporter> {
        super::reporter(self.kind(Env::detect()))
    }

    /// The options for the run called `id`, with `--restart` applied.
    pub fn options(&self, id: impl Into<String>) -> RunOptions {
        RunOptions::new(id).restart(self.restart)
    }
}
