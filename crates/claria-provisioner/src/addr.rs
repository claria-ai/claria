use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

/// Composite key for addressing a resource in state.
///
/// Two resources of the same type but different names (e.g. two
/// `bedrock_model_agreement` entries) have distinct addresses.
///
/// Serialized as the single string `resource_type.resource_name`, because this
/// is the key type of the `resources` map in the state file and JSON object
/// keys are strings. A derived struct impl makes `serde_json` refuse the whole
/// document with "key must be a string", which silently turned every state
/// flush into an error the moment the map held anything at all.
///
/// Deliberately not `specta::Type`: nothing puts an address on the IPC surface,
/// and a derived binding would describe an object while the wire carries a
/// string. Anything that needs one in TypeScript should declare it as a string.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct ResourceAddr {
    pub resource_type: String,
    pub resource_name: String,
}

impl fmt::Display for ResourceAddr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}.{}", self.resource_type, self.resource_name)
    }
}

impl FromStr for ResourceAddr {
    type Err = String;

    /// Splits on the first `.` only: `bedrock_model_agreement.anthropic.claude`
    /// is the `anthropic.claude` resource, not a type called `anthropic`.
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let (resource_type, resource_name) = raw.split_once('.').ok_or_else(|| {
            format!("resource address {raw:?} is not in resource_type.resource_name form")
        })?;
        if resource_type.is_empty() || resource_name.is_empty() {
            return Err(format!("resource address {raw:?} has an empty half"));
        }
        Ok(Self {
            resource_type: resource_type.to_string(),
            resource_name: resource_name.to_string(),
        })
    }
}

impl Serialize for ResourceAddr {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ResourceAddr {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // A key with no `.` is the v1 format, keyed by bare resource type. The
        // error is what sends `persistence::load` down its migration path.
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(de::Error::custom)
    }
}
