mod package;
mod worksheet;

use std::path::Path;

use dotall_core::{PatchedOutput, Result, ValidatedEdit};

// v0 writes string edits as inlineStr so sharedStrings.xml remains byte-identical.
pub(crate) fn apply(source: &Path, edit: &ValidatedEdit) -> Result<PatchedOutput> {
    package::patch(source, edit)
}
