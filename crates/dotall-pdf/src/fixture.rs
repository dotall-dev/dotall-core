pub fn minimal_form_pdf() -> Vec<u8> {
    let content = content_object(4, "BT /F1 24 Tf 10 150 Td (Hello) Tj ET\n");
    assemble(
        &[
            "1 0 obj<< /Type /Catalog /Pages 2 0 R /AcroForm 6 0 R >>endobj\n",
            "2 0 obj<< /Type /Pages /Kids [3 0 R] /Count 1 >>endobj\n",
            "3 0 obj<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R /Resources<< /Font<< /F1 5 0 R >> >> /Annots [7 0 R] >>endobj\n",
            content.as_str(),
            "5 0 obj<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>endobj\n",
            "6 0 obj<< /Fields [8 0 R] /NeedAppearances true >>endobj\n",
            "7 0 obj<< /Type /Annot /Subtype /Widget /Rect [10 10 120 40] /P 3 0 R /Parent 8 0 R /F 4 >>endobj\n",
            "8 0 obj<< /FT /Tx /T (Name) /V (Ada) /Kids [7 0 R] >>endobj\n",
        ],
        None,
    )
}

/// Checkbox (`/Btn`) with export values `Yes` / `Off`, initially Off.
pub fn minimal_checkbox_pdf() -> Vec<u8> {
    let content = content_object(4, "BT /F1 24 Tf 10 150 Td (Hello) Tj ET\n");
    assemble(
        &[
            "1 0 obj<< /Type /Catalog /Pages 2 0 R /AcroForm 6 0 R >>endobj\n",
            "2 0 obj<< /Type /Pages /Kids [3 0 R] /Count 1 >>endobj\n",
            "3 0 obj<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R /Resources<< /Font<< /F1 5 0 R >> >> /Annots [7 0 R] >>endobj\n",
            content.as_str(),
            "5 0 obj<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>endobj\n",
            "6 0 obj<< /Fields [8 0 R] /NeedAppearances true >>endobj\n",
            "7 0 obj<< /Type /Annot /Subtype /Widget /Rect [10 50 30 70] /P 3 0 R /Parent 8 0 R /F 4 /AS /Off /AP << /N << /Yes 9 0 R /Off 10 0 R >> >> >>endobj\n",
            "8 0 obj<< /FT /Btn /T (Agree) /V /Off /Kids [7 0 R] >>endobj\n",
            "9 0 obj<< /Type /XObject /Subtype /Form /BBox [0 0 20 20] /Length 0 >>stream\nendstream\nendobj\n",
            "10 0 obj<< /Type /XObject /Subtype /Form /BBox [0 0 20 20] /Length 0 >>stream\nendstream\nendobj\n",
        ],
        None,
    )
}

/// Intake form: Name + Email text, Agree checkbox, Department choice, `/Info` metadata.
///
/// Page content lists the form title and field labels so `read.page` returns meaningful text.
pub fn demo_form_pdf() -> Vec<u8> {
    let page_text = "\
BT
/F1 18 Tf 20 330 Td (Vendor Intake Form) Tj
0 -28 Td /F1 12 Tf (Name) Tj
0 -40 Td (Email) Tj
0 -50 Td (Agree to terms) Tj
0 -60 Td (Department) Tj
ET
";
    let content = content_object(4, page_text);
    assemble(
        &[
            "1 0 obj<< /Type /Catalog /Pages 2 0 R /AcroForm 6 0 R >>endobj\n",
            "2 0 obj<< /Type /Pages /Kids [3 0 R] /Count 1 >>endobj\n",
            "3 0 obj<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 360] /Contents 4 0 R /Resources<< /Font<< /F1 5 0 R >> >> /Annots [7 0 R 9 0 R 11 0 R 15 0 R] >>endobj\n",
            content.as_str(),
            "5 0 obj<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>endobj\n",
            "6 0 obj<< /Fields [8 0 R 10 0 R 12 0 R 16 0 R] /NeedAppearances true >>endobj\n",
            "7 0 obj<< /Type /Annot /Subtype /Widget /Rect [80 270 280 295] /P 3 0 R /Parent 8 0 R /F 4 >>endobj\n",
            "8 0 obj<< /FT /Tx /T (Name) /V (Ada Lovelace) /Kids [7 0 R] >>endobj\n",
            "9 0 obj<< /Type /Annot /Subtype /Widget /Rect [80 230 280 255] /P 3 0 R /Parent 10 0 R /F 4 >>endobj\n",
            "10 0 obj<< /FT /Tx /T (Email) /V (ada@example.com) /Kids [9 0 R] >>endobj\n",
            "11 0 obj<< /Type /Annot /Subtype /Widget /Rect [80 180 100 200] /P 3 0 R /Parent 12 0 R /F 4 /AS /Off /AP << /N << /Yes 13 0 R /Off 14 0 R >> >> >>endobj\n",
            "12 0 obj<< /FT /Btn /T (Agree) /V /Off /Kids [11 0 R] >>endobj\n",
            "13 0 obj<< /Type /XObject /Subtype /Form /BBox [0 0 20 20] /Length 0 >>stream\nendstream\nendobj\n",
            "14 0 obj<< /Type /XObject /Subtype /Form /BBox [0 0 20 20] /Length 0 >>stream\nendstream\nendobj\n",
            "15 0 obj<< /Type /Annot /Subtype /Widget /Rect [80 120 280 145] /P 3 0 R /Parent 16 0 R /F 4 >>endobj\n",
            "16 0 obj<< /FT /Ch /T (Department) /V (Engineering) /Opt [(Engineering) (Sales) (Operations)] /Kids [15 0 R] >>endobj\n",
            "17 0 obj<< /Title (Vendor Intake Form) /Author (Dotall Demo) /Subject (Vendor onboarding) /Creator (dotall-pdf) /Producer (dotall-pdf) >>endobj\n",
        ],
        Some(17),
    )
}

/// Build `N 0 obj<< /Length … >>stream…endstream\nendobj\n` with a correct byte length.
fn content_object(number: u32, stream: &str) -> String {
    format!(
        "{number} 0 obj<< /Length {} >>stream\n{stream}endstream\nendobj\n",
        stream.len()
    )
}

fn assemble(objects: &[&str], info_object: Option<usize>) -> Vec<u8> {
    let mut body = String::from("%PDF-1.4\n%\u{00e2}\u{00e3}\u{00cf}\u{00d3}\n");
    let mut offsets = Vec::new();
    for object in objects {
        offsets.push(body.len());
        body.push_str(object);
    }
    let xref_at = body.len();
    let mut xref = format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1);
    for offset in offsets {
        xref.push_str(&format!("{offset:010} 00000 n \n"));
    }
    let info = info_object
        .map(|number| format!(" /Info {number} 0 R"))
        .unwrap_or_default();
    let trailer = format!(
        "trailer<< /Size {} /Root 1 0 R{info} >>\nstartxref\n{xref_at}\n%%EOF\n",
        objects.len() + 1
    );
    let mut bytes = body.into_bytes();
    bytes.extend_from_slice(xref.as_bytes());
    bytes.extend_from_slice(trailer.as_bytes());
    bytes
}

#[cfg(test)]
mod tests {
    use super::{content_object, demo_form_pdf, minimal_form_pdf};
    use crate::parser::parse_pdf_bytes;

    #[test]
    fn content_object_length_matches_stream_bytes() {
        let stream = "BT (Hi) Tj ET\n";
        let object = content_object(4, stream);
        let declared: usize = object
            .split_once("/Length ")
            .and_then(|(_, rest)| rest.split_once(' '))
            .and_then(|(value, _)| value.parse().ok())
            .expect("length");
        assert_eq!(declared, stream.len());
    }

    #[test]
    fn demo_form_page_text_is_meaningful() {
        let model = parse_pdf_bytes(&demo_form_pdf()).expect("parse");
        let text = &model.pages[0].text;
        assert!(text.contains("Vendor Intake Form"), "{text}");
        assert!(text.contains("Name"), "{text}");
        assert!(text.contains("Email"), "{text}");
        assert!(text.contains("Agree to terms"), "{text}");
        assert!(text.contains("Department"), "{text}");
    }

    #[test]
    fn minimal_form_still_extracts_hello() {
        let model = parse_pdf_bytes(&minimal_form_pdf()).expect("parse");
        assert!(model.pages[0].text.contains("Hello"));
    }
}
