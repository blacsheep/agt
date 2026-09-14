use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

use crate::CategoryName;

pub const PACKAGE_COMPONENT_MAX_LEN: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackageRefComponent {
    Scope,
    Name,
}

impl fmt::Display for PackageRefComponent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scope => f.write_str("scope"),
            Self::Name => f.write_str("package name"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PackageRefParseError {
    #[error("package reference 不能为空")]
    EmptyReference,

    #[error("package reference `{reference}` 缺少 scope")]
    MissingScope { reference: String },

    #[error("package reference `{reference}` 缺少 package name")]
    MissingName { reference: String },

    #[error("package reference `{reference}` 包含额外的 `/`")]
    UnexpectedSlash { reference: String },

    #[error("{component} `{value}` 长度为 {length}，最大允许 {PACKAGE_COMPONENT_MAX_LEN} 个字符")]
    ComponentTooLong {
        component: PackageRefComponent,
        value: String,
        length: usize,
    },

    #[error(
        "{component} `{value}` 的第 {index} 个字符 `{character}` 无效；只允许小写 ASCII 字母、数字和连字符"
    )]
    InvalidCharacter {
        component: PackageRefComponent,
        value: String,
        index: usize,
        character: char,
    },

    #[error("{component} `{value}` 的连字符不能位于首尾或连续出现")]
    InvalidHyphenPlacement {
        component: PackageRefComponent,
        value: String,
    },

    #[error("package reference `{reference}` 使用了尚未支持的 version selector `{selector}`")]
    VersionSelectorUnsupported { reference: String, selector: String },
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PackageScope(String);

impl PackageScope {
    pub fn new(value: impl Into<String>) -> Result<Self, PackageRefParseError> {
        let value = value.into();
        validate_component(&value, PackageRefComponent::Scope)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl AsRef<str> for PackageScope {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for PackageScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PackageScope {
    type Err = PackageRefParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl TryFrom<String> for PackageScope {
    type Error = PackageRefParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for PackageScope {
    type Error = PackageRefParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl Serialize for PackageScope {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for PackageScope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_from_str(deserializer)
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PackageName(String);

impl PackageName {
    pub fn new(value: impl Into<String>) -> Result<Self, PackageRefParseError> {
        let value = value.into();
        validate_component(&value, PackageRefComponent::Name)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl AsRef<str> for PackageName {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for PackageName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PackageName {
    type Err = PackageRefParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl TryFrom<String> for PackageName {
    type Error = PackageRefParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for PackageName {
    type Error = PackageRefParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl Serialize for PackageName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for PackageName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_from_str(deserializer)
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PackageRef {
    scope: Option<PackageScope>,
    name: PackageName,
}

impl PackageRef {
    pub fn new(scope: Option<PackageScope>, name: PackageName) -> Self {
        Self { scope, name }
    }

    pub fn unscoped(name: PackageName) -> Self {
        Self::new(None, name)
    }

    pub fn scoped(scope: PackageScope, name: PackageName) -> Self {
        Self::new(Some(scope), name)
    }

    pub fn scope(&self) -> Option<&PackageScope> {
        self.scope.as_ref()
    }

    pub fn name(&self) -> &PackageName {
        &self.name
    }

    pub fn is_scoped(&self) -> bool {
        self.scope.is_some()
    }

    pub fn into_parts(self) -> (Option<PackageScope>, PackageName) {
        (self.scope, self.name)
    }
}

impl fmt::Display for PackageRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(scope) = &self.scope {
            write!(f, "@{scope}/")?;
        }
        self.name.fmt(f)
    }
}

impl FromStr for PackageRef {
    type Err = PackageRefParseError;

    fn from_str(reference: &str) -> Result<Self, Self::Err> {
        if reference.is_empty() {
            return Err(PackageRefParseError::EmptyReference);
        }

        if let Some(scoped) = reference.strip_prefix('@') {
            return parse_scoped_reference(reference, scoped);
        }

        if reference.contains('/') {
            return Err(PackageRefParseError::UnexpectedSlash {
                reference: reference.to_owned(),
            });
        }

        let name = package_without_version(reference, reference)?;
        Ok(Self::unscoped(PackageName::new(name)?))
    }
}

impl TryFrom<String> for PackageRef {
    type Error = PackageRefParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl TryFrom<&str> for PackageRef {
    type Error = PackageRefParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl Serialize for PackageRef {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for PackageRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_from_str(deserializer)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ResourceRef {
    #[serde(default)]
    category: CategoryName,
    package: PackageRef,
}

impl ResourceRef {
    pub fn persisted(package: PackageRef) -> Self {
        Self::persisted_in(CategoryName::default(), package)
    }

    pub fn persisted_in(category: CategoryName, package: PackageRef) -> Self {
        Self { category, package }
    }

    pub fn category(&self) -> &CategoryName {
        &self.category
    }

    pub fn package(&self) -> &PackageRef {
        &self.package
    }
}

impl From<PackageRef> for ResourceRef {
    fn from(package: PackageRef) -> Self {
        Self::persisted(package)
    }
}

impl fmt::Display for ResourceRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.category, self.package)
    }
}

fn parse_scoped_reference(
    reference: &str,
    scoped: &str,
) -> Result<PackageRef, PackageRefParseError> {
    let Some((scope, package)) = scoped.split_once('/') else {
        return Err(if scoped.is_empty() {
            PackageRefParseError::MissingScope {
                reference: reference.to_owned(),
            }
        } else {
            PackageRefParseError::MissingName {
                reference: reference.to_owned(),
            }
        });
    };

    if scope.is_empty() {
        return Err(PackageRefParseError::MissingScope {
            reference: reference.to_owned(),
        });
    }
    if package.is_empty() {
        return Err(PackageRefParseError::MissingName {
            reference: reference.to_owned(),
        });
    }
    if package.contains('/') {
        return Err(PackageRefParseError::UnexpectedSlash {
            reference: reference.to_owned(),
        });
    }

    let package = package_without_version(reference, package)?;
    Ok(PackageRef::scoped(
        PackageScope::new(scope)?,
        PackageName::new(package)?,
    ))
}

fn package_without_version<'a>(
    reference: &str,
    package: &'a str,
) -> Result<&'a str, PackageRefParseError> {
    if let Some((package, selector)) = package.split_once('@') {
        if package.is_empty() {
            return Err(PackageRefParseError::MissingName {
                reference: reference.to_owned(),
            });
        }
        return Err(PackageRefParseError::VersionSelectorUnsupported {
            reference: reference.to_owned(),
            selector: selector.to_owned(),
        });
    }
    Ok(package)
}

fn validate_component(
    value: &str,
    component: PackageRefComponent,
) -> Result<(), PackageRefParseError> {
    if value.is_empty() {
        return Err(match component {
            PackageRefComponent::Scope => PackageRefParseError::MissingScope {
                reference: value.to_owned(),
            },
            PackageRefComponent::Name => PackageRefParseError::MissingName {
                reference: value.to_owned(),
            },
        });
    }

    let length = value.chars().count();
    if length > PACKAGE_COMPONENT_MAX_LEN {
        return Err(PackageRefParseError::ComponentTooLong {
            component,
            value: value.to_owned(),
            length,
        });
    }

    if value.starts_with('-') || value.ends_with('-') || value.contains("--") {
        return Err(PackageRefParseError::InvalidHyphenPlacement {
            component,
            value: value.to_owned(),
        });
    }

    if let Some((index, character)) = value.chars().enumerate().find(|(_, character)| {
        !character.is_ascii_lowercase() && !character.is_ascii_digit() && *character != '-'
    }) {
        return Err(PackageRefParseError::InvalidCharacter {
            component,
            value: value.to_owned(),
            index,
            character,
        });
    }

    Ok(())
}

fn deserialize_from_str<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr,
    T::Err: fmt::Display,
{
    let value = String::deserialize(deserializer)?;
    value.parse().map_err(D::Error::custom)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashSet};

    use super::*;

    #[test]
    fn parses_and_displays_unscoped_reference() {
        let reference: PackageRef = "code-review".parse().unwrap();

        assert!(!reference.is_scoped());
        assert_eq!(reference.scope(), None);
        assert_eq!(reference.name().as_str(), "code-review");
        assert_eq!(reference.to_string(), "code-review");
    }

    #[test]
    fn parses_and_displays_scoped_reference() {
        let reference: PackageRef = "@acme/code-review".parse().unwrap();

        assert!(reference.is_scoped());
        assert_eq!(reference.scope().map(PackageScope::as_str), Some("acme"));
        assert_eq!(reference.name().as_str(), "code-review");
        assert_eq!(reference.to_string(), "@acme/code-review");
    }

    #[test]
    fn accepts_component_length_boundaries() {
        let max = "a".repeat(PACKAGE_COMPONENT_MAX_LEN);

        assert!(PackageName::new("a").is_ok());
        assert!(PackageScope::new("a").is_ok());
        assert!(PackageName::new(&max).is_ok());
        assert!(PackageScope::new(&max).is_ok());
    }

    #[test]
    fn rejects_components_longer_than_limit() {
        let too_long = "a".repeat(PACKAGE_COMPONENT_MAX_LEN + 1);

        assert!(matches!(
            PackageName::new(&too_long),
            Err(PackageRefParseError::ComponentTooLong {
                component: PackageRefComponent::Name,
                length: 65,
                ..
            })
        ));
        assert!(matches!(
            PackageScope::new(&too_long),
            Err(PackageRefParseError::ComponentTooLong {
                component: PackageRefComponent::Scope,
                length: 65,
                ..
            })
        ));
    }

    #[test]
    fn rejects_invalid_characters_in_scope_and_name() {
        for reference in [
            "Code-review",
            "code_review",
            "code.review",
            "code review",
            "code\\review",
            "../code-review",
            "@Acme/code-review",
            "@acme_org/code-review",
            "@acme.org/code-review",
            "@acme org/code-review",
            "@acme\\org/code-review",
        ] {
            assert!(reference.parse::<PackageRef>().is_err(), "{reference}");
        }
    }

    #[test]
    fn rejects_invalid_hyphen_placement() {
        for reference in [
            "-code",
            "code-",
            "code--review",
            "@-acme/code",
            "@acme-/code",
            "@acme--org/code",
        ] {
            assert!(matches!(
                reference.parse::<PackageRef>(),
                Err(PackageRefParseError::InvalidHyphenPlacement { .. })
            ));
        }
    }

    #[test]
    fn rejects_missing_components_and_extra_slashes() {
        assert_eq!(
            "".parse::<PackageRef>(),
            Err(PackageRefParseError::EmptyReference)
        );
        assert!(matches!(
            "@".parse::<PackageRef>(),
            Err(PackageRefParseError::MissingScope { .. })
        ));
        assert!(matches!(
            "@/code".parse::<PackageRef>(),
            Err(PackageRefParseError::MissingScope { .. })
        ));
        assert!(matches!(
            "@acme".parse::<PackageRef>(),
            Err(PackageRefParseError::MissingName { .. })
        ));
        assert!(matches!(
            "@acme/".parse::<PackageRef>(),
            Err(PackageRefParseError::MissingName { .. })
        ));
        assert!(matches!(
            "acme/code".parse::<PackageRef>(),
            Err(PackageRefParseError::UnexpectedSlash { .. })
        ));
        assert!(matches!(
            "@acme/code/extra".parse::<PackageRef>(),
            Err(PackageRefParseError::UnexpectedSlash { .. })
        ));
    }

    #[test]
    fn reports_version_selectors_as_typed_unsupported_errors() {
        for (reference, expected_selector) in [
            ("code-review@1.2.0", "1.2.0"),
            ("@acme/code-review@^1.2", "^1.2"),
            ("code-review@", ""),
        ] {
            assert!(matches!(
                reference.parse::<PackageRef>(),
                Err(PackageRefParseError::VersionSelectorUnsupported {
                    selector,
                    ..
                }) if selector == expected_selector
            ));
        }
    }

    #[test]
    fn canonical_display_round_trips_through_parser() {
        for input in ["code-review", "@acme/code-review", "@a/a-1"] {
            let parsed: PackageRef = input.parse().unwrap();
            let reparsed: PackageRef = parsed.to_string().parse().unwrap();

            assert_eq!(reparsed, parsed);
        }
    }

    #[test]
    fn exact_identities_do_not_create_short_name_aliases() {
        let unscoped: PackageRef = "code-review".parse().unwrap();
        let acme: PackageRef = "@acme/code-review".parse().unwrap();
        let platform: PackageRef = "@platform/code-review".parse().unwrap();

        let ordered = BTreeSet::from([platform.clone(), unscoped.clone(), acme.clone()]);
        assert_eq!(
            ordered.into_iter().collect::<Vec<_>>(),
            vec![unscoped.clone(), acme.clone(), platform.clone()]
        );

        let hashed = HashSet::from([unscoped, acme, platform]);
        assert_eq!(hashed.len(), 3);

        let scoped_only = HashSet::from([
            "@acme/code-review".parse::<PackageRef>().unwrap(),
            "@platform/code-review".parse::<PackageRef>().unwrap(),
        ]);
        assert!(!scoped_only.contains(&"code-review".parse::<PackageRef>().unwrap()));
    }

    #[test]
    fn package_ref_serde_uses_canonical_string_and_validates_input() {
        let reference: PackageRef = "@acme/code-review".parse().unwrap();

        assert_eq!(
            serde_json::to_string(&reference).unwrap(),
            r#""@acme/code-review""#
        );
        assert_eq!(
            serde_json::from_str::<PackageRef>(r#""@acme/code-review""#).unwrap(),
            reference
        );
        assert!(serde_json::from_str::<PackageRef>(r#""Code_review""#).is_err());
    }

    #[test]
    fn resource_ref_preserves_category_and_package() {
        let reference = ResourceRef::persisted_in(
            CategoryName::new("team").unwrap(),
            "@acme/review".parse().unwrap(),
        );
        let json = serde_json::to_string(&reference).unwrap();
        assert_eq!(
            serde_json::from_str::<ResourceRef>(&json).unwrap(),
            reference
        );
        assert_eq!(reference.to_string(), "team/@acme/review");
    }
}
