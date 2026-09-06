use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{McpId, PackageName};

const TOOL_TARGET_ENCODING_VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolSafetyAnnotations {
    pub read_only_hint: Option<bool>,
    pub destructive_hint: Option<bool>,
    pub open_world_hint: Option<bool>,
}

impl ToolSafetyAnnotations {
    pub const fn explicit(
        read_only_hint: bool,
        destructive_hint: bool,
        open_world_hint: bool,
    ) -> Self {
        Self {
            read_only_hint: Some(read_only_hint),
            destructive_hint: Some(destructive_hint),
            open_world_hint: Some(open_world_hint),
        }
    }

    pub const fn is_complete(self) -> bool {
        self.read_only_hint.is_some()
            && self.destructive_hint.is_some()
            && self.open_world_hint.is_some()
    }

    pub const fn is_known_safe_read(self) -> bool {
        matches!(self.read_only_hint, Some(true))
            && !matches!(self.destructive_hint, Some(true))
            && !matches!(self.open_world_hint, Some(true))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ToolId(String);

impl ToolId {
    pub fn from_target(target: &ToolTarget) -> Self {
        Self(target.canonical_id())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl AsRef<str> for ToolId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for ToolId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolTarget {
    Builtin {
        namespace: PackageName,
        name: PackageName,
    },
    Mcp {
        server: McpId,
        tool: String,
    },
}

impl ToolTarget {
    pub fn builtin(namespace: PackageName, name: PackageName) -> Self {
        Self::Builtin { namespace, name }
    }

    pub fn mcp(server: McpId, tool: impl Into<String>) -> Self {
        Self::Mcp {
            server,
            tool: tool.into(),
        }
    }

    pub fn id(&self) -> ToolId {
        ToolId::from_target(self)
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut encoded = vec![TOOL_TARGET_ENCODING_VERSION];
        match self {
            Self::Builtin { namespace, name } => {
                encoded.push(0);
                push_component(&mut encoded, namespace.as_str().as_bytes());
                push_component(&mut encoded, name.as_str().as_bytes());
            }
            Self::Mcp { server, tool } => {
                encoded.push(1);
                push_component(&mut encoded, server.as_str().as_bytes());
                push_component(&mut encoded, tool.as_bytes());
            }
        }
        encoded
    }

    fn canonical_id(&self) -> String {
        match self {
            Self::Builtin { namespace, name } => format!("builtin:{namespace}/{name}"),
            Self::Mcp { server, tool } => format!(
                "mcp:{}:{}:{}:{}",
                server.as_str().len(),
                server,
                tool.len(),
                tool
            ),
        }
    }
}

impl fmt::Display for ToolTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Builtin { namespace, name } => write!(f, "builtin:{namespace}/{name}"),
            Self::Mcp { server, tool } => write!(f, "mcp:{server}/{tool}"),
        }
    }
}

fn push_component(output: &mut Vec<u8>, value: &[u8]) {
    output.extend_from_slice(&(value.len() as u64).to_be_bytes());
    output.extend_from_slice(value);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(value: &str) -> PackageName {
        value.parse().expect("valid package name")
    }

    #[test]
    fn builtin_identity_has_stable_display_and_encoding() {
        let target = ToolTarget::builtin(package("skill"), package("load"));

        assert_eq!(target.to_string(), "builtin:skill/load");
        assert_eq!(target.id().to_string(), "builtin:skill/load");
        assert_eq!(target.canonical_bytes()[..2], [1, 0]);
    }

    #[test]
    fn mcp_identity_preserves_exact_raw_name() {
        let raw_name = "读取/文件__原样";
        let target = ToolTarget::mcp(McpId::new("filesystem"), raw_name);
        let id = target.id();

        assert!(
            id.as_str()
                .ends_with(&format!("{}:{raw_name}", raw_name.len()))
        );
        assert_eq!(target.to_string(), "mcp:filesystem/读取/文件__原样");
        assert_eq!(target.canonical_bytes()[..2], [1, 1]);
    }

    #[test]
    fn serde_round_trip_keeps_structured_target() {
        let target = ToolTarget::mcp(McpId::new("helper"), "a__b/c");
        let encoded = serde_json::to_string(&target).expect("serialize target");
        let decoded: ToolTarget = serde_json::from_str(&encoded).expect("deserialize target");

        assert_eq!(decoded, target);
        assert!(encoded.contains(r#""kind":"mcp""#));
    }

    #[test]
    fn length_prefix_prevents_structural_aliases() {
        let left = ToolTarget::mcp(McpId::new("a"), "b__c");
        let right = ToolTarget::mcp(McpId::new("a__b"), "c");

        assert_ne!(left.id(), right.id());
        assert_ne!(left.canonical_bytes(), right.canonical_bytes());
    }

    #[test]
    fn missing_annotations_are_not_known_safe() {
        assert!(!ToolSafetyAnnotations::default().is_known_safe_read());
        assert!(ToolSafetyAnnotations::explicit(true, false, false).is_known_safe_read());
        assert!(!ToolSafetyAnnotations::explicit(true, true, false).is_known_safe_read());
        assert!(!ToolSafetyAnnotations::explicit(true, false, true).is_known_safe_read());
    }
}
