use serde_derive::{Deserialize, Serialize};
use crate::models::duration::*;

/// Represents the definition of a timeout
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimeoutDefinition{

    /// Gets/sets the name of the timeout to use, if any
    #[serde(rename = "use", skip_serializing_if = "Option::is_none")]
    pub use_: Option<String>,

    /// Gets/sets the duration after which to timeout
    #[serde(rename = "after")]
    pub after : OneOfDurationOrIso8601Expression

}


/// Represents a value that can be either a TimeoutDefinition or a reference to a TimeoutDefinition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OneOfTimeoutDefinitionOrReference{
    /// Variant holding a timeout
    Timeout(TimeoutDefinition),
    /// Variant holding a reference to the timeout to use
    Reference(String)
}
impl Default for OneOfTimeoutDefinitionOrReference {
    fn default() -> Self {
        // Choose a default variant
        OneOfTimeoutDefinitionOrReference::Timeout(TimeoutDefinition::default())
    }
}