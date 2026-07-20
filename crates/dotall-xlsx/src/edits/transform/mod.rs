//! Pure A1 coordinate transformations for structural worksheet edits.

pub mod address;
pub mod formula;
pub mod sqref;

pub use address::{
    Axis, AxisChange, CellRef, RangeRef, TransformResult, parse_cell, parse_range, transform_cell,
    transform_range,
};
pub use formula::transform_formula;
pub use sqref::{SqrefError, transform_sqref};
