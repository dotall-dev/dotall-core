use serde::{Deserialize, Serialize};

use crate::model::CellValue;

pub const SCHEMA_ID: &str = "xlsx.cell-edits";
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum XlsxEditOp {
    SetCellValue {
        sheet: String,
        address: String,
        element_id: String,
        value: EditableValue,
    },
    SetCellFormula {
        sheet: String,
        address: String,
        element_id: String,
        formula: String,
    },
    SetRange {
        sheet: String,
        start_cell: String,
        values: Vec<Vec<EditableCell>>,
    },
    InsertRow {
        sheet: String,
        at: u32,
        count: u32,
    },
    DeleteRow {
        sheet: String,
        at: u32,
        count: u32,
    },
    InsertColumn {
        sheet: String,
        at: u32,
        count: u32,
    },
    DeleteColumn {
        sheet: String,
        at: u32,
        count: u32,
    },
    AddSheet {
        name: String,
        after: Option<String>,
    },
    RenameSheet {
        from: String,
        to: String,
    },
    DeleteSheet {
        name: String,
        #[serde(default)]
        dependency_policy: DeleteSheetPolicy,
    },
    MergeCells {
        sheet: String,
        range: String,
    },
    UnmergeCells {
        sheet: String,
        range: String,
    },
    SetColumnWidth {
        sheet: String,
        column: String,
        width: f64,
    },
    SetRowHeight {
        sheet: String,
        row: u32,
        height: f64,
    },
    FreezePanes {
        sheet: String,
        /// Openpyxl-style freeze cell; `None` clears freeze panes.
        cell: Option<String>,
    },
    DefineName {
        name: String,
        /// OOXML defined-name formula body (no leading `=`).
        formula: String,
    },
    DeleteName {
        name: String,
    },
    HideSheet {
        sheet: String,
        hidden: bool,
    },
    SetTabColor {
        sheet: String,
        /// AARRGGBB hex (with or without leading FF); `None` clears tab color.
        color: Option<String>,
    },
    SetAutoFilter {
        sheet: String,
        /// A1 range (`A1:D10`); `None` clears the auto filter.
        range: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeleteSheetPolicy {
    #[default]
    RejectIfReferenced,
    ReplaceReferencesWithRefError,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum EditableValue {
    String(String),
    Number(f64),
    Boolean(bool),
    Blank,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum EditableCell {
    Value(EditableValue),
    Formula(String),
}

impl EditableValue {
    pub fn to_cell_value(&self) -> CellValue {
        match self {
            Self::String(value) => CellValue::String(value.clone()),
            Self::Number(value) => CellValue::Float(*value),
            Self::Boolean(value) => CellValue::Boolean(*value),
            Self::Blank => CellValue::Empty,
        }
    }
}

pub fn format_cell_value(value: &CellValue) -> Option<String> {
    match value {
        CellValue::Empty => None,
        CellValue::String(value) => Some(value.clone()),
        CellValue::Float(value) => Some(value.to_string()),
        CellValue::Integer(value) => Some(value.to_string()),
        CellValue::Boolean(value) => Some(value.to_string()),
        CellValue::Error(value) => Some(value.clone()),
        CellValue::Datetime(value) => Some(value.clone()),
    }
}

pub fn format_editable_value(value: &EditableValue) -> Option<String> {
    match value {
        EditableValue::Blank => None,
        EditableValue::String(value) => Some(value.clone()),
        EditableValue::Number(value) => Some(value.to_string()),
        EditableValue::Boolean(value) => Some(value.to_string()),
    }
}
