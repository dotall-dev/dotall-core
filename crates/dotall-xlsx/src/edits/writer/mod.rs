mod package;
mod shared_strings;
mod worksheet;

use std::path::Path;

use dotall_core::{PatchedOutput, Result, ValidatedEdit};

// Workbooks with shared strings retain that storage strategy; inline strings remain
// the fallback for packages without an SST.
pub(crate) fn apply(source: &Path, edit: &ValidatedEdit) -> Result<PatchedOutput> {
    package::patch(source, edit)
}
