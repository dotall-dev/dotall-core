mod atomic;

// Re-exported for the store layer (Task 5+); unit tests live in `atomic`.
#[allow(unused_imports)]
pub(crate) use atomic::write_json;
