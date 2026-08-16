//! Write `demo/form.pdf` without building the full CLI workspace.
//!
//! ```bash
//! cargo run -p dotall-pdf --example write_demo_form -- demo/form.pdf
//! ```

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "demo/form.pdf".into());
    std::fs::write(&path, dotall_pdf::demo_form_pdf()).expect("write form.pdf");
    println!("wrote {path}");
}
