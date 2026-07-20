pub const SERVER_NAME: &str = "dotall";

pub mod error;
pub mod params;
pub mod response;

#[cfg(test)]
mod tests {
    use super::SERVER_NAME;

    #[test]
    fn server_name_is_stable() {
        assert_eq!(SERVER_NAME, "dotall");
    }
}
