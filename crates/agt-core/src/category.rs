use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

pub const DEFAULT_CATEGORY: &str = "default";

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CategoryName(String);

impl CategoryName {
    pub fn new(value: impl Into<String>) -> Result<Self, CategoryNameError> {
        let value = value.into();
        validate_category_name(&value)?;
        Ok(Self(value))
    }

    pub fn default_category() -> Self {
        Self(DEFAULT_CATEGORY.to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }

    pub fn is_default(&self) -> bool {
        self.0 == DEFAULT_CATEGORY
    }
}

impl Default for CategoryName {
    fn default() -> Self {
        Self::default_category()
    }
}

impl AsRef<str> for CategoryName {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for CategoryName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<String> for CategoryName {
    type Error = CategoryNameError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for CategoryName {
    type Error = CategoryNameError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl Serialize for CategoryName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CategoryName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(D::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error(
    "category 名称无效：{name}。首字符必须是 ASCII 字母或数字，后续只能包含 ASCII 字母、数字、'-'、'_'"
)]
pub struct CategoryNameError {
    name: String,
}

impl CategoryNameError {
    pub fn name(&self) -> &str {
        &self.name
    }
}

pub fn validate_category_name(name: &str) -> Result<(), CategoryNameError> {
    let bytes = name.as_bytes();
    let valid = bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes
            .iter()
            .skip(1)
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));

    if valid {
        Ok(())
    } else {
        Err(CategoryNameError {
            name: name.to_owned(),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct QualifiedRef<T> {
    category: CategoryName,
    name: T,
}

impl<T> QualifiedRef<T> {
    pub fn new(category: CategoryName, name: T) -> Self {
        Self { category, name }
    }

    pub fn in_default(name: T) -> Self {
        Self::new(CategoryName::default(), name)
    }

    pub fn category(&self) -> &CategoryName {
        &self.category
    }

    pub fn name(&self) -> &T {
        &self.name
    }

    pub fn into_parts(self) -> (CategoryName, T) {
        (self.category, self.name)
    }
}

impl<T: fmt::Display> fmt::Display for QualifiedRef<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.category, self.name)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::AgentId;

    #[test]
    fn accepts_safe_category_names() {
        for name in [
            "default", "internal", "custom", "tmp", "team-1", "exp_2", "A1",
        ] {
            let category = CategoryName::new(name).expect("safe category should pass");
            assert_eq!(category.as_str(), name);
        }
    }

    #[test]
    fn rejects_unsafe_category_names() {
        for name in [
            "",
            ".",
            "..",
            "-team",
            "_team",
            "team/name",
            "team.name",
            "team name",
        ] {
            assert!(CategoryName::new(name).is_err(), "{name} should fail");
        }
    }

    #[test]
    fn category_deserialization_validates_input() {
        let error = serde_json::from_value::<CategoryName>(json!("../internal"))
            .expect_err("unsafe category should fail");

        assert!(error.to_string().contains("category 名称无效"));
    }

    #[test]
    fn qualified_reference_round_trips_as_structured_data() {
        let reference = QualifiedRef::new(
            CategoryName::new("team").unwrap(),
            AgentId::new("reviewer.v2"),
        );
        let value = serde_json::to_value(&reference).unwrap();

        assert_eq!(value, json!({"category": "team", "name": "reviewer.v2"}));
        assert_eq!(
            serde_json::from_value::<QualifiedRef<AgentId>>(value).unwrap(),
            reference
        );
        assert_eq!(reference.to_string(), "team/reviewer.v2");
    }
}
