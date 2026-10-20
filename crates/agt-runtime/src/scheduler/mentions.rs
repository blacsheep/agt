//! Mentions written in an agent's final reply. Same syntax as the Web console:
//! `@id` for simple identities and `@["id"]` for any other, after the start of the
//! text, whitespace or an opening bracket.

const OPENERS: &[char] = &['（', '(', '【', '['];
const TERMINATORS: &[char] = &[
    '，', '。', '！', '？', '、', '：', '；', ',', '!', '?', ';', ':', '(', ')', '（', '）', '【',
    '】', '[', ']',
];

/// Identities named in `body`, in order of appearance, without duplicates.
pub(super) fn written(body: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut previous: Option<char> = None;
    let mut rest = body;
    while let Some(at) = rest.find('@') {
        let before = rest[..at].chars().next_back().or(previous);
        let after = &rest[at + 1..];
        let boundary = before.is_none_or(|c| c.is_whitespace() || OPENERS.contains(&c));
        let name = if !boundary {
            None
        } else if let Some(literal) = after.strip_prefix('[') {
            quoted(literal)
        } else {
            let end = after
                .find(|c: char| c.is_whitespace() || c == '@' || TERMINATORS.contains(&c))
                .unwrap_or(after.len());
            Some(after[..end].to_owned()).filter(|n| !n.is_empty())
        };
        if let Some(name) = name
            && !names.contains(&name)
        {
            names.push(name);
        }
        previous = Some('@');
        rest = after;
    }
    names
}

/// The identity in `"…"]`, as a JSON string literal.
fn quoted(literal: &str) -> Option<String> {
    if !literal.starts_with('"') {
        return None;
    }
    let mut escaped = false;
    for (i, c) in literal.char_indices().skip(1) {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '"' => {
                return literal[i + 1..]
                    .starts_with(']')
                    .then(|| serde_json::from_str(&literal[..=i]).ok())
                    .flatten();
            }
            _ => {}
        }
    }
    None
}

/// Mentions to attach to a reply: written identities that are active members, other
/// than the sender. Agents cannot mention everyone, so `all` is never a mention here.
/// A trailing full stop, as in "ask @reviewer.", is not part of the name.
pub(super) fn resolve(body: &str, sender: &str, members: &[String]) -> Vec<String> {
    let member = |name: &str| members.iter().any(|m| m == name) && name != sender;
    written(body)
        .into_iter()
        .filter_map(|name| {
            if member(&name) {
                Some(name)
            } else {
                let trimmed = name.trim_end_matches(['.', '\'', '"']);
                member(trimmed).then(|| trimmed.to_owned())
            }
        })
        .filter(|name| name != "all")
        .fold(Vec::new(), |mut found, name| {
            if !found.contains(&name) {
                found.push(name);
            }
            found
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn members() -> Vec<String> {
        ["god", "p01", "p12", "role:with space", "reviewer", "all"]
            .map(str::to_owned)
            .to_vec()
    }

    #[test]
    fn written_mentions_follow_the_console_syntax() {
        assert_eq!(
            resolve("请 @p01 发言，然后@p12（补充）", "god", &members()),
            vec!["p01"],
            "a mention needs a boundary before the @"
        );
        assert_eq!(
            resolve("(@p12) 和 @[\"role:with space\"] 请看", "god", &members()),
            vec!["p12", "role:with space"]
        );
        assert_eq!(
            resolve("Over to you, @reviewer.", "god", &members()),
            vec!["reviewer"]
        );
    }

    #[test]
    fn non_members_self_everyone_and_mail_addresses_are_not_mentions() {
        let body = "@god @all @stranger mail me at a@p01.example @p01 @p01";
        assert_eq!(resolve(body, "god", &members()), vec!["p01"]);
        assert!(resolve("@[\"unterminated", "god", &members()).is_empty());
    }
}
