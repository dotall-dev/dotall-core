pub const FORMAT_ID: &str = "xlsx";

#[cfg(test)]
mod tests {
    use super::FORMAT_ID;

    #[test]
    fn format_id_is_stable() {
        assert_eq!(FORMAT_ID, "xlsx");
    }
}
