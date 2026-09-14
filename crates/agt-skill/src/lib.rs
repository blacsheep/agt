pub mod config;
pub mod diagnostic;
pub mod error;
pub mod name;
pub mod safe;
pub mod store;

pub use config::{
    AGT_PACKAGE_METADATA_KEY, MAX_SKILL_COMPATIBILITY_CHARS, MAX_SKILL_DESCRIPTION_CHARS,
    SkillCreateOptions, SkillDocument, SkillMetadata,
};
pub use diagnostic::{SkillDiagnostic, SkillDiagnosticCategory};
pub use error::{ImportCleanupFailure, ImportCleanupReport, Result, SkillError};
pub use name::validate_skill_name;
pub use safe::{
    SkillBatchResult, SkillDeleteResult, SkillDiagnosticSummary, SkillDocumentResult,
    SkillImportResult, SkillScanResult, SkillSummary,
};
pub use store::{
    MAX_IMPORT_ENTRIES, MAX_IMPORT_RELATIVE_DEPTH, MAX_IMPORT_SINGLE_FILE_BYTES,
    MAX_IMPORT_TOTAL_BYTES, SkillScan, SkillStore, StoredSkill, StoredSkillMetadata,
};

pub const SKILL_DOCUMENT_MAX_BYTES: u64 = 256 * 1024;
