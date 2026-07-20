use dotall_mcp::server::FlushOnClose;

#[test]
fn flush_on_close_defaults_to_enabled() {
    assert!(FlushOnClose::resolve(false, None).enabled());
}

#[test]
fn no_flush_flag_disables_flush_on_close() {
    assert!(!FlushOnClose::resolve(true, None).enabled());
}

#[test]
fn zero_value_in_environment_disables_flush_on_close() {
    assert!(!FlushOnClose::resolve(false, Some("0")).enabled());
}

#[test]
fn no_flush_flag_wins_over_environment_enable() {
    assert!(!FlushOnClose::resolve(true, Some("1")).enabled());
}
