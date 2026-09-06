use std::collections::BTreeMap;
use std::path::Path;

use agt_core::{PackageName, PackageRef};
use serde::{Deserialize, Serialize};

use crate::error::{Result, SkillError};

pub const SKILL_FILE: &str = "SKILL.md";
pub const AGT_PACKAGE_METADATA_KEY: &str = "agt.package";
pub const MAX_SKILL_DESCRIPTION_CHARS: usize = 1024;
pub const MAX_SKILL_COMPATIBILITY_CHARS: usize = 500;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillMetadata {
    pub name: PackageName,
    pub description: String,
    pub license: Option<String>,
    pub compatibility: Option<String>,
    pub metadata: BTreeMap<String, String>,
    pub allowed_tools: Option<String>,
}

impl SkillMetadata {
    pub fn new(name: impl AsRef<str>, description: impl Into<String>) -> Result<Self> {
        let name =
            PackageName::try_from(name.as_ref()).map_err(|source| SkillError::InvalidName {
                name: name.as_ref().to_string(),
                source: Box::new(source),
            })?;
        Self::for_package(PackageRef::unscoped(name), description)
    }

    pub fn for_package(package: PackageRef, description: impl Into<String>) -> Result<Self> {
        let mut metadata = BTreeMap::new();
        if package.is_scoped() {
            metadata.insert(AGT_PACKAGE_METADATA_KEY.to_string(), package.to_string());
        }
        let value = Self {
            name: package.name().clone(),
            description: description.into(),
            license: None,
            compatibility: None,
            metadata,
            allowed_tools: None,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn package_ref(&self) -> Result<PackageRef> {
        let Some(raw_package) = self.metadata.get(AGT_PACKAGE_METADATA_KEY) else {
            return Ok(PackageRef::unscoped(self.name.clone()));
        };
        let package =
            raw_package
                .parse::<PackageRef>()
                .map_err(|source| SkillError::InvalidConfig {
                    message: format!("metadata.{AGT_PACKAGE_METADATA_KEY} 无效：{source}"),
                })?;
        if !package.is_scoped() {
            return Err(SkillError::InvalidConfig {
                message: format!(
                    "metadata.{AGT_PACKAGE_METADATA_KEY} 只用于 scoped ref；unscoped skill 应省略该字段"
                ),
            });
        }
        if package.name() != &self.name {
            return Err(SkillError::InvalidConfig {
                message: format!(
                    "metadata.{AGT_PACKAGE_METADATA_KEY} leaf {} 与 frontmatter name {} 不一致",
                    package.name(),
                    self.name
                ),
            });
        }
        Ok(package)
    }

    pub fn validate(&self) -> Result<()> {
        validate_required_text(
            "description",
            &self.description,
            MAX_SKILL_DESCRIPTION_CHARS,
        )?;
        if let Some(license) = &self.license
            && license.trim().is_empty()
        {
            return Err(SkillError::InvalidConfig {
                message: "license 存在时不能为空".to_string(),
            });
        }
        if let Some(compatibility) = &self.compatibility {
            validate_required_text(
                "compatibility",
                compatibility,
                MAX_SKILL_COMPATIBILITY_CHARS,
            )?;
        }
        self.package_ref()?;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillDocument {
    pub metadata: SkillMetadata,
    pub body: String,
}

impl SkillDocument {
    pub fn new(metadata: SkillMetadata, body: impl Into<String>) -> Result<Self> {
        metadata.validate()?;
        Ok(Self {
            metadata,
            body: body.into(),
        })
    }

    pub fn default_for(metadata: SkillMetadata) -> Result<Self> {
        let body = format!(
            "# {}\n\n请描述这个 skill 的使用场景、触发条件和执行步骤。\n",
            metadata.name
        );
        Self::new(metadata, body)
    }

    pub fn parse(path: impl AsRef<Path>, text: &str) -> Result<Self> {
        let path = path.as_ref();
        let (frontmatter, body) =
            split_frontmatter(text).map_err(|message| SkillError::InvalidConfig {
                message: format!("{}：{}", path.display(), message),
            })?;
        let metadata = parse_metadata_yaml(path, frontmatter)?;
        Self::new(metadata, body.to_string())
    }

    pub fn parse_metadata(path: impl AsRef<Path>, text: &str) -> Result<SkillMetadata> {
        let path = path.as_ref();
        let (frontmatter, _) =
            split_frontmatter(text).map_err(|message| SkillError::InvalidConfig {
                message: format!("{}：{}", path.display(), message),
            })?;
        parse_metadata_yaml(path, frontmatter)
    }

    pub fn render(&self) -> Result<String> {
        self.metadata.validate()?;
        let yaml =
            serde_yaml::to_string(&SkillMetadataYaml::from(&self.metadata)).map_err(|source| {
                SkillError::Yaml {
                    path: Path::new(SKILL_FILE).to_path_buf(),
                    source,
                }
            })?;
        Ok(format!("---\n{}---\n{}", yaml, self.body))
    }
}

fn parse_metadata_yaml(path: &Path, frontmatter: &str) -> Result<SkillMetadata> {
    let yaml: SkillMetadataYaml =
        serde_yaml::from_str(frontmatter).map_err(|source| SkillError::yaml(path, source))?;
    SkillMetadata::try_from(yaml)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct SkillMetadataYaml {
    name: Option<String>,
    description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    compatibility: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    metadata: BTreeMap<String, String>,
    #[serde(
        default,
        rename = "allowed-tools",
        skip_serializing_if = "Option::is_none"
    )]
    allowed_tools: Option<String>,
}

impl TryFrom<SkillMetadataYaml> for SkillMetadata {
    type Error = SkillError;

    fn try_from(value: SkillMetadataYaml) -> Result<Self> {
        let name = value.name.ok_or_else(|| SkillError::InvalidConfig {
            message: "name 不能为空".to_string(),
        })?;
        let name =
            PackageName::try_from(name.as_str()).map_err(|source| SkillError::InvalidName {
                name: name.clone(),
                source: Box::new(source),
            })?;
        let description = value.description.ok_or_else(|| SkillError::InvalidConfig {
            message: "description 不能为空".to_string(),
        })?;
        let metadata = Self {
            name,
            description,
            license: value.license,
            compatibility: value.compatibility,
            metadata: value.metadata,
            allowed_tools: value.allowed_tools,
        };
        metadata.validate()?;
        Ok(metadata)
    }
}

impl From<&SkillMetadata> for SkillMetadataYaml {
    fn from(value: &SkillMetadata) -> Self {
        Self {
            name: Some(value.name.to_string()),
            description: Some(value.description.clone()),
            license: value.license.clone(),
            compatibility: value.compatibility.clone(),
            metadata: value.metadata.clone(),
            allowed_tools: value.allowed_tools.clone(),
        }
    }
}

fn split_frontmatter(text: &str) -> std::result::Result<(&str, &str), &'static str> {
    let first_line_end = text
        .find('\n')
        .ok_or("SKILL.md 必须以 YAML frontmatter 开头")?;
    let first_line = text[..first_line_end].trim_end_matches('\r');
    if first_line != "---" {
        return Err("SKILL.md 必须以 YAML frontmatter 开头");
    }

    let frontmatter_start = first_line_end + 1;
    let mut line_start = frontmatter_start;
    for line in text[frontmatter_start..].split_inclusive('\n') {
        let line_without_newline = line.trim_end_matches(&['\r', '\n'][..]);
        if line_without_newline == "---" {
            let frontmatter = &text[frontmatter_start..line_start];
            let body_start = line_start + line.len();
            return Ok((frontmatter, &text[body_start..]));
        }
        line_start += line.len();
    }

    Err("SKILL.md 缺少 frontmatter 结束分隔符")
}

fn validate_required_text(field: &str, value: &str, max_chars: usize) -> Result<()> {
    if value.trim().is_empty() {
        return Err(SkillError::InvalidConfig {
            message: format!("{field} 不能为空"),
        });
    }
    let length = value.chars().count();
    if length > max_chars {
        return Err(SkillError::InvalidConfig {
            message: format!("{field} 长度为 {length}，最大允许 {max_chars} characters"),
        });
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillCreateOptions {
    pub package: PackageRef,
    pub description: String,
}

impl SkillCreateOptions {
    pub fn new(package: impl AsRef<str>, description: impl Into<String>) -> Result<Self> {
        let raw = package.as_ref();
        let package =
            raw.parse::<PackageRef>()
                .map_err(|source| SkillError::InvalidPackageRef {
                    reference: raw.to_string(),
                    source: Box::new(source),
                })?;
        let options = Self {
            package,
            description: description.into(),
        };
        options.metadata()?;
        Ok(options)
    }

    pub fn metadata(&self) -> Result<SkillMetadata> {
        SkillMetadata::for_package(self.package.clone(), self.description.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_and_parses_standard_optional_fields() {
        let mut metadata = SkillMetadata::for_package(
            "@acme/code-review".parse().expect("package"),
            "Review code changes. Use when asked to inspect diffs.",
        )
        .expect("metadata");
        metadata.license = Some("MIT".to_string());
        metadata.compatibility = Some("Requires git.".to_string());
        metadata
            .metadata
            .insert("owner".to_string(), "platform".to_string());
        metadata.allowed_tools = Some("Read Grep".to_string());
        let document = SkillDocument::new(metadata, "# Code Review\n\nSteps.\n").expect("document");

        let rendered = document.render().expect("render");
        let parsed = SkillDocument::parse(SKILL_FILE, &rendered).expect("parse");

        assert_eq!(
            parsed.metadata.package_ref().unwrap().to_string(),
            "@acme/code-review"
        );
        assert_eq!(parsed.metadata.license.as_deref(), Some("MIT"));
        assert_eq!(
            parsed.metadata.compatibility.as_deref(),
            Some("Requires git.")
        );
        assert_eq!(
            parsed.metadata.metadata.get("owner").map(String::as_str),
            Some("platform")
        );
        assert_eq!(parsed.metadata.allowed_tools.as_deref(), Some("Read Grep"));
        assert_eq!(parsed.body, "# Code Review\n\nSteps.\n");
    }

    #[test]
    fn unknown_top_level_fields_are_tolerated() {
        let text =
            "---\nname: code-review\ndescription: Review.\nfuture-field: value\n---\n# Skill\n";
        let parsed = SkillDocument::parse(SKILL_FILE, text).expect("parse");

        assert_eq!(parsed.metadata.name.as_str(), "code-review");
    }

    #[test]
    fn parses_metadata_without_loading_body() {
        let text = "---\nname: code-review\ndescription: Review.\n---\n# Skill\n\nBody.\n";
        let metadata = SkillDocument::parse_metadata(SKILL_FILE, text).expect("metadata");

        assert_eq!(metadata.name.as_str(), "code-review");
        assert_eq!(metadata.description, "Review.");
    }

    #[test]
    fn validates_scoped_package_metadata() {
        let valid = "---\nname: code-review\ndescription: Review.\nmetadata:\n  agt.package: '@acme/code-review'\n---\n# Skill\n";
        assert!(SkillDocument::parse(SKILL_FILE, valid).is_ok());

        for invalid in [
            "---\nname: code-review\ndescription: Review.\nmetadata:\n  agt.package: code-review\n---\n# Skill\n",
            "---\nname: code-review\ndescription: Review.\nmetadata:\n  agt.package: '@acme/debug'\n---\n# Skill\n",
            "---\nname: code-review\ndescription: Review.\nmetadata:\n  agt.package: '@Acme/code-review'\n---\n# Skill\n",
        ] {
            assert!(SkillDocument::parse(SKILL_FILE, invalid).is_err());
        }
    }

    #[test]
    fn enforces_description_and_compatibility_limits() {
        assert!(SkillMetadata::new("a", "a").is_ok());
        assert!(SkillMetadata::new("a", "a".repeat(MAX_SKILL_DESCRIPTION_CHARS)).is_ok());
        assert!(SkillMetadata::new("a", "a".repeat(MAX_SKILL_DESCRIPTION_CHARS + 1)).is_err());

        let mut metadata = SkillMetadata::new("a", "description").unwrap();
        metadata.compatibility = Some("a".to_string());
        assert!(metadata.validate().is_ok());
        metadata.compatibility = Some("a".repeat(MAX_SKILL_COMPATIBILITY_CHARS));
        assert!(metadata.validate().is_ok());
        metadata.compatibility = Some("a".repeat(MAX_SKILL_COMPATIBILITY_CHARS + 1));
        assert!(metadata.validate().is_err());
    }

    #[test]
    fn rejects_missing_or_empty_required_fields() {
        for text in [
            "# Skill\n",
            "---\nname: code-review\n---\n# Skill\n",
            "---\nname: code-review\ndescription: ''\n---\n# Skill\n",
        ] {
            assert!(SkillDocument::parse(SKILL_FILE, text).is_err());
        }
    }

    #[test]
    fn rejects_invalid_frontmatter_separator() {
        let text = "---\nname: code-review\ndescription: Review.\n--- nope\n# Skill\n";

        assert!(SkillDocument::parse(SKILL_FILE, text).is_err());
    }

    #[test]
    fn allowed_tools_is_preserved_as_an_experimental_raw_string() {
        let text =
            "---\nname: code-review\ndescription: Review.\nallowed-tools: ''\n---\n# Skill\n";
        let document = SkillDocument::parse(SKILL_FILE, text).unwrap();

        assert_eq!(document.metadata.allowed_tools.as_deref(), Some(""));
        assert!(document.render().unwrap().contains("allowed-tools: ''"));
    }
}
