mod auto_filter;
mod dimensions;
mod freeze_panes;
mod merges;
mod package;
mod page_margins;
mod page_orientation;
mod print_scale;
mod shared_strings;
mod structural;
mod tab_color;
mod workbook;
mod worksheet;

use std::path::Path;

use dotall_core::{PatchedOutput, Result, ValidatedEdit};

// Workbooks with shared strings retain that storage strategy; inline strings remain
// the fallback for packages without an SST.
pub(crate) fn apply(source: &Path, edit: &ValidatedEdit) -> Result<PatchedOutput> {
    package::patch(source, edit)
}

pub(crate) fn validate_rename_safety(package: &[u8], from: &str) -> Result<()> {
    workbook::validate_rename_safety(package, from)
}

pub(crate) fn delete_sheet_references(
    package: &[u8],
    name: &str,
) -> Result<workbook::DeleteSheetReferences> {
    workbook::delete_sheet_references(package, name)
}

pub(crate) fn sheet_visibility(package: &[u8]) -> Result<Vec<(String, bool)>> {
    workbook::sheet_visibility(package)
}
