//! Work that blocks its thread, run where blocking is allowed: tokio's blocking pool, or, on wasm (which has no threads
//! to hand it to), right where it is called.
use std::any::Any;
use std::fmt;

/// Why a blocking call produced no value.
pub(crate) enum Error {
    /// The code panicked; this is what it panicked with.
    Panicked(Box<dyn Any + Send>),
    /// The runtime stopped the call before it ran (only the blocking pool does).
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    Cancelled,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Panicked(_) => f.write_str("the blocking call panicked"),
            Self::Cancelled => f.write_str("the blocking call was cancelled"),
        }
    }
}

/// Runs `f` on the blocking pool and waits for it, inside the current span and with the current subscriber (a pool
/// thread has neither of its own), so what it logs or traces stays where it was called.
#[cfg(not(target_family = "wasm"))]
pub(crate) async fn run<F, R>(f: F) -> Result<R, Error>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    tokio::task::spawn_blocking(crate::api::observe::carry(f))
        .await
        .map_err(|error| {
            if error.is_panic() {
                Error::Panicked(error.into_panic())
            } else {
                Error::Cancelled
            }
        })
}

/// Runs `f` here: wasm has one thread, so nothing else runs until it returns. (Wasm builds abort on a panic unless they
/// are built to unwind, so a panic in `f` ends the program there; the catch is for builds that do unwind.)
#[cfg(target_family = "wasm")]
#[allow(clippy::unused_async, reason = "the same signature as the pool's")]
pub(crate) async fn run<F, R>(f: F) -> Result<R, Error>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).map_err(Error::Panicked)
}

#[cfg(feature = "yaml")]
/// Reads a whole file: on the blocking pool, or, on wasm, right here. (Instead of `tokio::fs`, which wasm does not have.)
pub(crate) async fn read_to_string(path: impl AsRef<std::path::Path>) -> std::io::Result<String> {
    let path = path.as_ref().to_path_buf();
    match run(move || std::fs::read_to_string(path)).await {
        Ok(read) => read,
        Err(error) => Err(std::io::Error::other(error.to_string())),
    }
}
