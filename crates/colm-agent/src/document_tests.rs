use super::*;
use std::io::Write;

struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new() -> Self {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "colm-document-test-{}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self, name: &str) -> std::path::PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn docx(path: &std::path::Path, document_xml: &str) {
    let file = std::fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("word/document.xml", options).unwrap();
    zip.write_all(document_xml.as_bytes()).unwrap();
    zip.finish().unwrap();
}

#[test]
fn pdf_text_extracts_the_repository_fixture() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/CoLM202X/preprocess/Forcings/Potential_spikes.pdf");
    let bytes = std::fs::read(&path).unwrap();
    let (pages, text, truncated) = pdf_text(&bytes).unwrap();
    assert!(pages >= 1, "{pages}");
    assert!(!truncated);
    assert!(text.contains("ERA5"), "{}", &text[..text.len().min(300)]);
}

#[test]
fn pdf_text_rejects_bytes_that_are_not_a_pdf() {
    let error = pdf_text(b"not a pdf").unwrap_err();
    assert!(
        error.to_string().contains("PDF") || error.to_string().contains("pdf"),
        "{error}"
    );
}

#[test]
fn docx_extracts_paragraphs_tables_and_entities() {
    let scratch = Scratch::new();
    let path = scratch.path("sample.docx");
    docx(
        &path,
        r#"<?xml version="1.0" encoding="UTF-8"?>
        <w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
          <w:p><w:r><w:t>Hello &amp; CoLM</w:t></w:r></w:p>
          <w:p><w:r><w:t>line </w:t><w:tab/><w:t>tabbed</w:t></w:r></w:p>
          <w:tbl><w:tr><w:tc><w:p><w:r><w:t>cell A</w:t></w:r></w:p></w:tc>
          <w:tc><w:p><w:r><w:t>cell B</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
        </w:body></w:document>"#,
    );
    let extracted = extract(&path).unwrap();
    assert_eq!(extracted.title.as_deref(), Some("sample.docx"));
    assert_eq!(extracted.pages, None);
    assert!(!extracted.truncated);
    assert!(
        extracted.text.contains("Hello & CoLM"),
        "{}",
        extracted.text
    );
    assert!(extracted.text.contains("line tabbed"), "{}", extracted.text);
    assert!(extracted.text.contains("cell A"), "{}", extracted.text);
}

#[test]
fn docx_without_document_xml_is_refused() {
    let scratch = Scratch::new();
    let path = scratch.path("broken.docx");
    let file = std::fs::File::create(&path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("word/styles.xml", options).unwrap();
    zip.write_all(b"<styles/>").unwrap();
    zip.finish().unwrap();
    let error = extract(&path).unwrap_err();
    assert!(error.to_string().contains("document.xml"), "{error}");
}

#[test]
fn plain_text_is_read_and_missing_types_are_refused() {
    let scratch = Scratch::new();
    let text = scratch.path("note.md");
    std::fs::write(&text, "第一行\n第二行\n").unwrap();
    let extracted = extract(&text).unwrap();
    assert!(extracted.text.contains("第一行"), "{}", extracted.text);
    let binary = scratch.path("photo.png");
    std::fs::write(&binary, b"png").unwrap();
    let error = extract(&binary).unwrap_err();
    assert!(error.to_string().contains("unsupported"), "{error}");
}

#[cfg(target_os = "macos")]
#[test]
fn legacy_doc_is_converted_with_the_system_converter() {
    if !std::path::Path::new("/usr/bin/textutil").exists() {
        return;
    }
    let scratch = Scratch::new();
    let source = scratch.path("legacy.txt");
    std::fs::write(&source, "旧版 Word 文档\n第二段\n").unwrap();
    let doc = scratch.path("legacy.doc");
    let status = std::process::Command::new("/usr/bin/textutil")
        .args(["-convert", "doc", "-output"])
        .arg(&doc)
        .arg(&source)
        .status()
        .unwrap();
    assert!(status.success());
    let extracted = extract(&doc).unwrap();
    assert!(
        extracted.text.contains("旧版 Word 文档"),
        "{}",
        extracted.text
    );
    assert!(!extracted.warnings.is_empty());
}
