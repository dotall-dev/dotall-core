use crate::dependencies::reference_spans;

use super::address::{AxisChange, RangeRef, TransformResult, transform_range};

/// Rewrites references that target the edited sheet while leaving all other
/// formula bytes unchanged. References deleted by the change become `#REF!`.
pub fn transform_formula(
    formula: &str,
    formula_sheet: &str,
    edited_sheet: &str,
    change: AxisChange,
) -> String {
    let mut rewritten = String::with_capacity(formula.len());
    let mut cursor = 0;

    for spanned in reference_spans(formula) {
        let reference = spanned.reference;
        let targets_edited_sheet = reference.sheet.as_deref().map_or_else(
            || formula_sheet.eq_ignore_ascii_case(edited_sheet),
            |sheet| sheet.eq_ignore_ascii_case(edited_sheet),
        );
        if !targets_edited_sheet {
            continue;
        }

        let range = RangeRef {
            start: reference.start.clone(),
            end: reference
                .end
                .clone()
                .unwrap_or_else(|| reference.start.clone()),
        };
        let transformed = transform_range(&range, change);
        let replacement = match transformed {
            TransformResult::Kept(range)
                if range
                    == RangeRef {
                        start: reference.start.clone(),
                        end: reference
                            .end
                            .clone()
                            .unwrap_or_else(|| reference.start.clone()),
                    } =>
            {
                continue;
            }
            TransformResult::Kept(range) => rewrite_reference(
                &formula[spanned.span.clone()],
                reference.end.is_some(),
                &range,
            ),
            TransformResult::Removed | TransformResult::RefError => "#REF!".into(),
        };
        rewritten.push_str(&formula[cursor..spanned.span.start]);
        rewritten.push_str(&replacement);
        cursor = spanned.span.end;
    }

    if cursor == 0 {
        formula.to_owned()
    } else {
        rewritten.push_str(&formula[cursor..]);
        rewritten
    }
}

fn rewrite_reference(source: &str, is_range: bool, range: &RangeRef) -> String {
    let original_start = source
        .rsplit_once('!')
        .map_or(source, |(_, address)| address)
        .split_once(':')
        .map_or_else(
            || {
                source
                    .rsplit_once('!')
                    .map_or(source, |(_, address)| address)
            },
            |(start, _)| start,
        );
    let suffix_length = original_start.len()
        + if is_range {
            source.rsplit_once(':').map_or(0, |(_, end)| end.len() + 1)
        } else {
            0
        };
    let prefix_length = source.len() - suffix_length;
    if is_range {
        format!("{}{}:{}", &source[..prefix_length], range.start, range.end)
    } else {
        format!("{}{}", &source[..prefix_length], range.start)
    }
}
