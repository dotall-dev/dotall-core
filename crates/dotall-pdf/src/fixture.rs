pub fn minimal_form_pdf() -> Vec<u8> {
    assemble(&[
        "1 0 obj<< /Type /Catalog /Pages 2 0 R /AcroForm 6 0 R >>endobj\n",
        "2 0 obj<< /Type /Pages /Kids [3 0 R] /Count 1 >>endobj\n",
        "3 0 obj<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R /Resources<< /Font<< /F1 5 0 R >> >> /Annots [7 0 R] >>endobj\n",
        "4 0 obj<< /Length 37 >>stream\nBT /F1 24 Tf 10 150 Td (Hello) Tj ET\nendstream\nendobj\n",
        "5 0 obj<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>endobj\n",
        "6 0 obj<< /Fields [8 0 R] /NeedAppearances true >>endobj\n",
        "7 0 obj<< /Type /Annot /Subtype /Widget /Rect [10 10 120 40] /P 3 0 R /Parent 8 0 R /F 4 >>endobj\n",
        "8 0 obj<< /FT /Tx /T (Name) /V (Ada) /Kids [7 0 R] >>endobj\n",
    ])
}

/// Checkbox (`/Btn`) with export values `Yes` / `Off`, initially Off.
pub fn minimal_checkbox_pdf() -> Vec<u8> {
    assemble(&[
        "1 0 obj<< /Type /Catalog /Pages 2 0 R /AcroForm 6 0 R >>endobj\n",
        "2 0 obj<< /Type /Pages /Kids [3 0 R] /Count 1 >>endobj\n",
        "3 0 obj<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R /Resources<< /Font<< /F1 5 0 R >> >> /Annots [7 0 R] >>endobj\n",
        "4 0 obj<< /Length 37 >>stream\nBT /F1 24 Tf 10 150 Td (Hello) Tj ET\nendstream\nendobj\n",
        "5 0 obj<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>endobj\n",
        "6 0 obj<< /Fields [8 0 R] /NeedAppearances true >>endobj\n",
        "7 0 obj<< /Type /Annot /Subtype /Widget /Rect [10 50 30 70] /P 3 0 R /Parent 8 0 R /F 4 /AS /Off /AP << /N << /Yes 9 0 R /Off 10 0 R >> >> >>endobj\n",
        "8 0 obj<< /FT /Btn /T (Agree) /V /Off /Kids [7 0 R] >>endobj\n",
        "9 0 obj<< /Type /XObject /Subtype /Form /BBox [0 0 20 20] /Length 0 >>stream\nendstream\nendobj\n",
        "10 0 obj<< /Type /XObject /Subtype /Form /BBox [0 0 20 20] /Length 0 >>stream\nendstream\nendobj\n",
    ])
}

fn assemble(objects: &[&str]) -> Vec<u8> {
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
    let trailer = format!(
        "trailer<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n",
        objects.len() + 1
    );
    let mut bytes = body.into_bytes();
    bytes.extend_from_slice(xref.as_bytes());
    bytes.extend_from_slice(trailer.as_bytes());
    bytes
}
