//! Fetching a document over HTTP(S): with reqwest, or, where there is no HTTP client (wasm), not at all.
use std::fmt;

/// Why a document could not be fetched. Displays as what to add after "failed to fetch <what> from <url>: ".
#[allow(
    dead_code,
    reason = "a build constructs only the variants its target can produce"
)]
pub(crate) enum Error {
    /// The request failed before there was a response.
    Request(String),
    /// The server answered with this status.
    Status(String),
    /// The body could not be read.
    Body(String),
    /// This build has no HTTP client.
    Unavailable,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Request(error) => f.write_str(error),
            Self::Status(status) => write!(f, "HTTP {status}"),
            Self::Body(error) => write!(f, "the response could not be read: {error}"),
            Self::Unavailable => f.write_str("HTTP is not available on wasm"),
        }
    }
}

/// The body of a successful GET.
#[cfg(not(target_family = "wasm"))]
pub(crate) async fn get_text(url: &str) -> Result<String, Error> {
    let response = reqwest::get(url)
        .await
        .map_err(|e| Error::Request(e.to_string()))?;
    if !response.status().is_success() {
        return Err(Error::Status(response.status().to_string()));
    }
    response
        .text()
        .await
        .map_err(|e| Error::Body(e.to_string()))
}

/// Wasm has no HTTP client here.
#[cfg(target_family = "wasm")]
#[allow(clippy::unused_async, reason = "the same signature as the native one")]
pub(crate) async fn get_text(_url: &str) -> Result<String, Error> {
    Err(Error::Unavailable)
}
