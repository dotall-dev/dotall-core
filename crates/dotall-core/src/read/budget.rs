/// Applies a character-based token estimate of four characters per token.
///
/// The continuation cursor is a UTF-8 byte offset into `content`; callers must
/// pass it back unchanged to resume the same rendered response.
pub fn apply_budget(
    content: &str,
    max_tokens: usize,
    offset: usize,
) -> crate::Result<(String, bool, Option<String>)> {
    if offset > content.len() || !content.is_char_boundary(offset) {
        return Err(crate::DotallError::InvalidArgument {
            reason: "continuation cursor is not a valid UTF-8 boundary".into(),
        });
    }
    let remaining = &content[offset..];
    let max_chars = max_tokens.saturating_mul(4).max(1);
    if remaining.chars().count() <= max_chars {
        return Ok((remaining.to_owned(), false, None));
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

    Ok((
        remaining[..preferred_end].to_owned(),
        true,
        Some(next.to_string()),
    ))
}

#[cfg(test)]
mod tests {
    use super::apply_budget;

    #[test]
    fn truncation_sets_a_cursor_and_resume_is_lossless() {
        let content = "row one\nrow two\nrow three\n";
        let (first, truncated, cursor) = apply_budget(content, 3, 0).expect("first");
        let (second, second_truncated, second_cursor) = apply_budget(
            content,
            20,
            cursor
                .as_deref()
                .expect("continuation")
                .parse()
                .expect("numeric cursor"),
        )
        .expect("second");

        assert!(truncated);
        assert!(!second_truncated);
        assert_eq!(second_cursor, None);
        assert_eq!(format!("{first}{second}"), content);
    }

    #[test]
    fn mid_character_offset_is_an_error() {
        let content = "é";
        assert_eq!(content.len(), 2);
        let err = apply_budget(content, 10, 1).expect_err("mid-char");
        assert!(err.to_string().contains("UTF-8"));
    }
}
