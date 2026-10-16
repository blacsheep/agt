//! Errors the console API returns to the page.
//!
//! Messages are the Chinese source text of the page's translation catalog
//! (`web/public/locales`), so they must stay byte-for-byte stable;
//! `web/test/i18n.test.mjs` scans `ApiError::new` calls for them.

use serde_json::{Value, json};

#[derive(Debug, Clone)]
pub(crate) struct ApiError {
    pub status: u16,
    pub code: String,
    pub message: String,
    pub field: Option<Value>,
}

impl ApiError {
    pub fn new(status: u16, code: &str, message: &str) -> Self {
        Self {
            status,
            code: code.to_owned(),
            message: message.to_owned(),
            field: None,
        }
    }

    pub fn with_field(mut self, field: Value) -> Self {
        self.field = Some(field);
        self
    }

    /// Anything unexpected. Never carries the underlying detail to the page.
    pub fn internal() -> Self {
        Self::new(500, "internal_error", "本地服务异常，请查看终端并刷新。")
    }

    pub fn body(&self) -> Value {
        let mut error = json!({ "code": self.code, "message": self.message });
        if let Some(field) = &self.field {
            error["field"] = field.clone();
        }
        json!({ "error": error })
    }
}
