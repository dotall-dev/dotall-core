pub const ALL_DIR_NAME: &str = ".all";

#[cfg(test)]
mod tests {
    use super::ALL_DIR_NAME;

    #[test]
    fn all_directory_name_is_stable() {
        assert_eq!(ALL_DIR_NAME, ".all");
    }
}
