pub mod dependencies;
pub mod detection;
pub mod edits;
pub mod format;
pub mod ids;
pub mod model;
pub mod parser;
pub mod projection;
pub mod selector;
pub mod structure;

pub const FORMAT_ID: &str = "xlsx";

pub use format::XlsxFormat;
pub use model::{
    CellModel, CellValue, NamedRange, PreservationStatus, SCHEMA_ID, SCHEMA_VERSION,
    SheetDimensions, SheetModel, StyleEntry, UnmodeledMap, WorkbookModel,
};
pub use parser::parse_workbook;

#[cfg(test)]
mod tests {
    use super::{FORMAT_ID, ids};

    #[test]
    fn format_id_is_stable() {
        assert_eq!(FORMAT_ID, "xlsx");
    }

    #[test]
    fn cell_ids_are_deterministic_and_opaque() {
        let first = ids::cell_id("Revenue", "A1", 1);
        let second = ids::cell_id("Revenue", "A1", 1);
        let different_address = ids::cell_id("Revenue", "B1", 1);

        assert_eq!(first, second);
        assert_ne!(first, different_address);
        assert!(first.starts_with("c_"));
        assert!(!first.contains("Revenue"));
        assert!(!first.contains("A1"));
    }

    #[test]
    fn sheet_and_workbook_ids_are_opaque() {
        let workbook = ids::workbook_id("source-content-hash", 1);
        let sheet = ids::sheet_id("Revenue", 0, 1);

        assert!(workbook.starts_with("wb_"));
        assert!(sheet.starts_with("sh_"));
        assert!(!sheet.contains("Revenue"));
    }

    #[test]
    fn column_names_follow_excel_conventions() {
        assert_eq!(super::model::column_name(0), "A");
        assert_eq!(super::model::column_name(25), "Z");
        assert_eq!(super::model::column_name(26), "AA");
        assert_eq!(super::model::column_name(701), "ZZ");
    }
}
