//! Payload codecs: transform what the engine writes to the store (encryption, compression).
//!
//! With a codec, every journal record, journal header and stored history payload is serialized to JSON, passed
//! through [`PayloadCodec::encode`], and stored as an envelope that names the codec:
//!
//! ```json
//! { "$jackdaw_codec": "aes-gcm", "data": "<base64>" }
//! ```
//!
//! Reading decodes it again. An engine that meets an envelope for a codec it was not built with fails with an error
//! that names the codec, instead of handing the encoded bytes to a workflow.
use super::error::Error;
use base64::Engine as _;
use serde_json::{Value, json};
use std::fmt;

const ENVELOPE: &str = "$jackdaw_codec";

/// Transforms the bytes the engine stores. Implementations must be deterministic in what they decode: `decode(encode(x))
/// == x`.
pub trait PayloadCodec: Send + Sync + fmt::Debug + 'static {
    /// A stable name, stored with every encoded payload.
    fn name(&self) -> &str;

    /// Encodes `plain` (JSON bytes) for storage.
    ///
    /// # Errors
    /// A message saying why the payload could not be encoded.
    fn encode(&self, plain: &[u8]) -> Result<Vec<u8>, String>;

    /// Reverses [`encode`](Self::encode).
    ///
    /// # Errors
    /// A message saying why the payload could not be decoded (for example a wrong key).
    fn decode(&self, encoded: &[u8]) -> Result<Vec<u8>, String>;
}

/// `value` as the store keeps it.
pub(crate) fn encode(codec: Option<&dyn PayloadCodec>, value: &Value) -> Result<Value, Error> {
    let Some(codec) = codec else {
        return Ok(value.clone());
    };
    let plain = serde_json::to_vec(value)
        .map_err(|e| Error::Engine(format!("a payload could not be serialized: {e}")))?;
    let encoded = codec.encode(&plain).map_err(|message| {
        Error::Engine(format!(
            "codec `{}` could not encode a payload: {message}",
            codec.name()
        ))
    })?;
    Ok(json!({
        ENVELOPE: codec.name(),
        "data": base64::engine::general_purpose::STANDARD.encode(encoded),
    }))
}

/// `value` as it was before [`encode`]. A value that is not an envelope is returned as it is.
pub(crate) fn decode(codec: Option<&dyn PayloadCodec>, value: Value) -> Result<Value, Error> {
    let Some(name) = value
        .get(ENVELOPE)
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return Ok(value);
    };
    let codec = codec.filter(|codec| codec.name() == name).ok_or_else(|| {
        Error::Engine(format!(
            "this payload was stored with codec `{name}`; build the engine with that codec to read it"
        ))
    })?;
    let data = value
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Engine(format!("a `{name}` payload has no data")))?;
    let encoded = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|e| Error::Engine(format!("a `{name}` payload is not valid base64: {e}")))?;
    let plain = codec.decode(&encoded).map_err(|message| {
        Error::Engine(format!(
            "codec `{name}` could not decode a payload: {message}"
        ))
    })?;
    serde_json::from_slice(&plain)
        .map_err(|e| Error::Engine(format!("a decoded `{name}` payload is not JSON: {e}")))
}
