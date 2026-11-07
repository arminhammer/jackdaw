//! A workflow's input, read from the sources its user chooses: files, environment variables and arguments.
//!
//! A workflow's input type (`#[run]`'s second parameter) describes everything a run needs up front. [`Layers`] builds
//! that value from any mix of sources, with `serde`'s defaults at the bottom. Sources are merged in the order they are
//! added, and a later source wins over an earlier one, key by key:
//!
//! ```
//! # fn main() -> Result<(), jackdaw::config::Error> {
//! use jackdaw::config::Layers;
//!
//! #[derive(serde::Deserialize)]
//! struct Pipeline {
//!     source: String,
//!     #[serde(default = "four")]
//!     workers: u8,
//! }
//! fn four() -> u8 { 4 }
//!
//! let pipeline: Pipeline = Layers::new()
//!     .file_if_exists("pipeline.toml")
//!     .env_from("PIPELINE_", [("PIPELINE_SOURCE".to_string(), "s3://bucket".to_string())])
//!     .load()?;
//! assert_eq!((pipeline.source.as_str(), pipeline.workers), ("s3://bucket", 4));
//! # Ok(()) }
//! ```
use figment::Figment;
use figment::providers::{Format, Json, Serialized, Toml};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::path::Path;

/// Why a value could not be read: a source that could not be read, or a value that is missing or has the wrong type.
/// The message names the setting and, for a file, the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    message: String,
}

impl Error {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

impl From<Error> for crate::api::Error {
    fn from(error: Error) -> Self {
        Self::invalid(error)
    }
}

/// A workflow's input type: everything a run needs, described once. Derive it with `#[derive(Config)]`.
///
/// The type is plain `serde`; the trait adds the command line flags it can be read from (see
/// [`Program`](crate::runner::Program)) and nothing else. A type that wants no flags implements `flags` as an empty list.
pub trait Config: Serialize + DeserializeOwned + Send + 'static {
    /// The flags, one per field that has one.
    fn flags() -> Vec<Flag>;
}

/// One command line flag of a [`Config`].
#[derive(Debug, Clone, Copy)]
pub struct Flag {
    /// The setting the flag gives: the field's name in files.
    pub key: &'static str,
    /// The flag's name without `--`.
    pub long: &'static str,
    /// The one-letter form.
    pub short: Option<char>,
    /// The field's doc comment.
    pub help: &'static str,
    /// How many values it takes.
    pub kind: FlagKind,
    /// Reads one value as text.
    pub parse: fn(&str) -> Result<serde_json::Value, String>,
}

/// How many values a [`Flag`] takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlagKind {
    /// `--name value`
    Value,
    /// `--name value`, as often as needed.
    List,
    /// `--name` is true, `--name=false` is false.
    Switch,
}

/// Reads `text` as a `T` and as the JSON value it serializes to. What `#[derive(Config)]` uses for a flag's parser.
///
/// # Errors
/// The message of `T`'s `FromStr`.
pub fn parse_value<T>(text: &str) -> Result<serde_json::Value, String>
where
    T: std::str::FromStr + Serialize,
    T::Err: std::fmt::Display,
{
    let value = text.parse::<T>().map_err(|e| e.to_string())?;
    serde_json::to_value(value).map_err(|e| e.to_string())
}

/// Sources of a value, merged in the order they are added.
#[derive(Debug, Default)]
#[must_use]
pub struct Layers {
    figment: Figment,
    errors: Vec<String>,
}

impl Layers {
    /// No sources yet: only `serde`'s defaults would apply.
    pub fn new() -> Self {
        Self::default()
    }

    /// A file, by its extension: `.toml`, `.json`, `.yaml` or `.yml`. It must exist.
    pub fn file(self, path: impl AsRef<Path>) -> Self {
        self.add_file(path.as_ref(), true)
    }

    /// As [`file`](Self::file), and the file is skipped if it does not exist.
    pub fn file_if_exists(self, path: impl AsRef<Path>) -> Self {
        self.add_file(path.as_ref(), false)
    }

    fn add_file(mut self, path: &Path, required: bool) -> Self {
        let shown = path.display();
        if !path.exists() {
            if required {
                self.errors.push(format!("the file {shown} does not exist"));
            }
            return self;
        }
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        match extension.as_deref() {
            Some("toml") => self.figment = self.figment.merge(Toml::file(path)),
            Some("json") => self.figment = self.figment.merge(Json::file(path)),
            Some("yaml" | "yml") => match read_yaml(path) {
                Ok(value) => self.figment = self.figment.merge(Serialized::defaults(value)),
                Err(e) => self.errors.push(format!("{shown}: {e}")),
            },
            other => self.errors.push(format!(
                "{shown}: unsupported extension `.{}` (use .toml, .json, .yaml or .yml)",
                other.unwrap_or_default()
            )),
        }
        self
    }

    /// The process's environment variables that start with `prefix` (compared without regard to case). The rest of
    /// the name, lowercased, is the setting's name, and `__` goes one level into a nested setting:
    /// `APP_LIMITS__RETRIES=5` sets `limits.retries`. Values are read as JSON scalars, arrays or objects (`8`, `true`, `[1, 2]`) or, failing that, as text.
    pub fn env(self, prefix: &str) -> Self {
        self.env_from(prefix, std::env::vars())
    }

    /// As [`env`](Self::env), for variables given by the caller (a test, or a program that has its own).
    pub fn env_from<I, K, V>(mut self, prefix: &str, vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let prefix = prefix.to_ascii_uppercase();
        for (name, value) in vars {
            let upper = name.as_ref().to_ascii_uppercase();
            let Some(key) = upper.strip_prefix(&prefix).filter(|k| !k.is_empty()) else {
                continue;
            };
            let key = key.to_ascii_lowercase().replace("__", ".");
            let value = env_value(value.as_ref());
            self.figment = self.figment.merge(Serialized::default(&key, value));
        }
        self
    }

    /// Values from anything `serde` can serialize to a map, such as parsed command line arguments. A source should
    /// leave out what the user did not give (`#[serde(skip_serializing_if = "Option::is_none")]`), or it would
    /// replace the sources below it.
    pub fn args(mut self, args: impl Serialize) -> Self {
        self.figment = self.figment.merge(Serialized::defaults(args));
        self
    }

    /// Merges the sources and reads the value.
    ///
    /// # Errors
    /// A file that is missing or cannot be read, and a setting that is missing or has the wrong type.
    pub fn load<T: DeserializeOwned>(self) -> Result<T, Error> {
        if !self.errors.is_empty() {
            return Err(Error::new(self.errors.join("; ")));
        }
        self.figment
            .extract()
            .map_err(|e| Error::new(e.to_string()))
    }
}

/// `8`, `1.5`, `true`, `[1, 2]` and `{"a": 1}` are what they look like; anything else is text.
fn env_value(text: &str) -> serde_json::Value {
    let text = text.trim();
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(value) if !value.is_string() && !value.is_null() => value,
        _ => serde_json::Value::String(text.to_string()),
    }
}

fn read_yaml(path: &Path) -> Result<serde_json::Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_saphyr::from_str(&text).map_err(|e| e.to_string())
}
