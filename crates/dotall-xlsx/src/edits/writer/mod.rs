mod package;
mod worksheet;

use std::path::Path;

use dotall_core::{PatchedOutput, Result, ValidatedEdit};

pub(crate) fn apply(source: &Path, edit: &ValidatedEdit) -> Result<PatchedOutput> {
    package::patch(source, edit)
}
