//! Chat control commands, as `AgtCommands.parse` in `web/public/commands.js`
//! reads them. The page runs them through dedicated endpoints; the server only
//! refuses to post one as an ordinary message by accident.

/// Whether `text` is a control command such as `/model gpt-5`, rather than
/// text that merely starts with a slash (`/tmp/x`, `//model`).
pub(crate) fn is_command(text: &str) -> bool {
    let value = text.trim();
    if value == "/" {
        return true;
    }
    if value.starts_with("//") {
        return false;
    }
    let Some(rest) = value.strip_prefix('/') else {
        return false;
    };
    let mut chars = rest.char_indices();
    if !chars.next().is_some_and(|(_, c)| c.is_ascii_alphabetic()) {
        return false;
    }
    let end = chars
        .find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '_' || *c == '-'))
        .map_or(rest.len(), |(i, _)| i);
    let arguments = &rest[end..];
    arguments.is_empty()
        || (arguments.starts_with([' ', '\t']) && !arguments.contains(['\r', '\n']))
}

#[cfg(test)]
mod tests {
    use super::is_command;

    #[test]
    fn commands_and_plain_text() {
        for text in [
            "/",
            "/model",
            " /Model gpt-5 ",
            "/new",
            "/anything-future",
            "/a\tb",
        ] {
            assert!(is_command(text), "{text}");
        }
        for text in [
            "//model",
            "/tmp/project",
            "Explain /model",
            "/model\nThis is quoted text",
            "/9lives",
            "/model:x",
            "",
        ] {
            assert!(!is_command(text), "{text}");
        }
    }
}
