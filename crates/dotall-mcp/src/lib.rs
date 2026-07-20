pub const SERVER_NAME: &str = "dotall";

#[cfg(test)]
mod tests {
    use super::SERVER_NAME;

    #[test]
    fn server_name_is_stable() {
        assert_eq!(SERVER_NAME, "dotall");
    }
}
