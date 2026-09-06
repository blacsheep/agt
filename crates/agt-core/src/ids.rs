use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! define_id {
    ($name:ident) => {
        #[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self::new(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self::new(value)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

define_id!(AgentId);
define_id!(EnvId);
define_id!(SkillId);
define_id!(McpId);
define_id!(KbId);
define_id!(ProviderId);
define_id!(ModelId);
define_id!(PromptId);
define_id!(SessionId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_keeps_original_value_for_display_and_as_ref() {
        let id = AgentId::new("coder");

        assert_eq!(id.as_str(), "coder");
        assert_eq!(id.as_ref(), "coder");
        assert_eq!(id.to_string(), "coder");
    }
}
