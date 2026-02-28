use std::fmt;

use serde::{Deserialize, Serialize};
use specta::Type;

/// Composite key for addressing a resource in state.
///
/// Two resources of the same type but different names (e.g. two
/// `bedrock_model_agreement` entries) have distinct addresses.
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize, Type)]
pub struct ResourceAddr {
    pub resource_type: String,
    pub resource_name: String,
}

impl fmt::Display for ResourceAddr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}.{}", self.resource_type, self.resource_name)
    }
}
