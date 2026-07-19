/// Applies a character-based token estimate of four characters per token.
///
/// The continuation cursor is a UTF-8 byte offset into `content`; callers must
/// pass it back unchanged to resume the same rendered response.
pub fn apply_budget(
    content: &str,
    max_tokens: usize,
    offset: usize,
) -> (String, bool, Option<String>) {
    let remaining = content.get(offset..).unwrap_or_default();
    let max_chars = max_tokens.saturating_mul(4).max(1);
    if remaining.chars().count() <= max_chars {
        return (remaining.to_owned(), false, None);
    }

    let mut end = 0;
    for (character_count, (index, character)) in remaining.char_indices().enumerate() {
        if character_count >= max_chars {
            break;
        }
        end = index + character.len_utf8();
    }
    let preferred_end = remaining[..end]
        .rfind('\n')
        .filter(|position| *position > max_chars / 2)
        .map_or(end, |position| position + 1);
    let next = offset + preferred_end;

    (
        remaining[..preferred_end].to_owned(),
        true,
        Some(next.to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::apply_budget;

    #[test]
    fn truncation_sets_a_cursor_and_resume_is_lossless() {
        let content = "row one\nrow two\nrow three\n";
        let (first, truncated, cursor) = apply_budget(content, 3, 0);
        let (second, second_truncated, second_cursor) = apply_budget(
            content,
            20,
            cursor
                .as_deref()
                .expect("continuation")
                .parse()
                .expect("numeric cursor"),
        );

        assert!(truncated);
        assert!(!second_truncated);
        assert_eq!(second_cursor, None);
        assert_eq!(format!("{first}{second}"), content);
    }
}
