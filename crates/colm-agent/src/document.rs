//! 本地文档附件：把用户选择的 PDF / DOCX / DOC / 纯文本抽成文字，供助手作为上下文读取。
//!
//! 规则（第 768 轮）：只在本地解析，原件不上传；PDF 只读文字层（扫描件明确报错），
//! DOCX 用 zip + `word/document.xml`，旧 `.doc` 仅在系统有 textutil/soffice 时尽力转成 DOCX。
//! 所有格式都有大小与字符上限，避免一个附件把 sidecar 或模型上下文拖死。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::Serialize;

/// PDF 原始大小上限（与 `fetch_url` 本地路径一致）。
pub(crate) const MAX_PDF_BYTES: u64 = 64 * 1024 * 1024;
/// 最多返回的 PDF 页数/字符数（与 `fetch_url` 一致）。
pub(crate) const MAX_PDF_PAGES: usize = 300;
pub(crate) const MAX_PDF_TEXT_CHARS: usize = 2_000_000;
/// DOCX/DOC 原始大小上限。
const MAX_OFFICE_BYTES: u64 = 64 * 1024 * 1024;
/// DOCX 里 `word/document.xml` 解压后最多读这么多字节（防 zip 炸弹；正常论文、报告只有几 MB）。
/// 测试里用 1 MB，免得为了验证上限去压一个 64 MB 的文件。
#[cfg(not(test))]
const MAX_DOCX_XML_BYTES: u64 = 64 * 1024 * 1024;
#[cfg(test)]
const MAX_DOCX_XML_BYTES: u64 = 1024 * 1024;
/// 纯文本文件原始大小上限。
const MAX_TEXT_BYTES: u64 = 20 * 1024 * 1024;
/// 所有附件最终最多返回的字符数（每条消息前端的合计上限另算）。
pub const MAX_DOCUMENT_CHARS: usize = 2_000_000;
/// 旧 .doc 转换超时；转换器卡住时不能拖死助手。
const DOC_CONVERT_TIMEOUT_SECS: u64 = 60;

/// 抽出来的文档。字段名直接给前端 JSON 用。
#[derive(Debug, Clone, Serialize)]
pub struct ExtractedDocument {
    pub title: Option<String>,
    pub text: String,
    /// PDF 的页数；其他格式为 null。
    pub pages: Option<usize>,
    pub truncated: bool,
    pub warnings: Vec<String>,
}

/// 按扩展名抽取文档文字。调用方负责让用户明确选择文件（GUI 文件对话框）。
pub fn extract(path: &Path) -> Result<ExtractedDocument> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .context("the selected file has no name")?;
    let extension = path
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "pdf" => extract_pdf(&path),
        "docx" => extract_docx(&path, name),
        "doc" => extract_legacy_doc(&path, name),
        "txt" | "text" | "md" | "markdown" | "csv" | "json" => extract_plain(&path, name),
        _ => bail!(
            "unsupported document type: .{extension} (supported: pdf, docx, doc, txt, md, csv, json)"
        ),
    }
}

fn file_size(path: &Path, limit: u64) -> Result<u64> {
    let metadata =
        std::fs::metadata(path).with_context(|| format!("cannot read {}", path.display()))?;
    if !metadata.is_file() {
        bail!("the selected path is not a file");
    }
    if metadata.len() > limit {
        bail!(
            "{} is larger than the {} MB attachment limit",
            path.display(),
            limit / 1024 / 1024
        );
    }
    Ok(metadata.len())
}

fn ensure_text(text: String) -> (String, bool) {
    let truncated = text.chars().count() > MAX_DOCUMENT_CHARS;
    if truncated {
        (text.chars().take(MAX_DOCUMENT_CHARS).collect(), true)
    } else {
        (text, false)
    }
}

fn extract_plain(path: &Path, name: String) -> Result<ExtractedDocument> {
    file_size(path, MAX_TEXT_BYTES)?;
    let bytes = std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes).trim().to_owned();
    if text.is_empty() {
        bail!("{name} has no readable text");
    }
    let (text, truncated) = ensure_text(text);
    Ok(ExtractedDocument {
        title: Some(name),
        text,
        pages: None,
        truncated,
        warnings: Vec::new(),
    })
}

fn extract_pdf(path: &Path) -> Result<ExtractedDocument> {
    file_size(path, MAX_PDF_BYTES)?;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .with_context(|| format!("cannot open {}", path.display()))?
        .take(MAX_PDF_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_PDF_BYTES {
        bail!("{} is larger than the PDF attachment limit", path.display());
    }
    let (pages, text, truncated) = pdf_text(&bytes)?;
    Ok(ExtractedDocument {
        title: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned()),
        text,
        pages: Some(pages),
        truncated,
        warnings: Vec::new(),
    })
}

fn extract_docx(path: &Path, name: String) -> Result<ExtractedDocument> {
    file_size(path, MAX_OFFICE_BYTES)?;
    let file =
        std::fs::File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file).context("not a valid DOCX (zip) file")?;
    // 解压出来的大小要设上限：几 MB 的 DOCX 可以把 document.xml 压成几 GB（zip 炸弹），
    // `read_to_string` 不设上限会把 sidecar 的内存撑爆。正文 XML 超过上限就截断并标记。
    let mut xml = String::new();
    let entry = archive
        .by_name("word/document.xml")
        .context("DOCX has no word/document.xml")?;
    let mut xml_bytes = Vec::new();
    entry
        .take(MAX_DOCX_XML_BYTES + 1)
        .read_to_end(&mut xml_bytes)
        .context("cannot read word/document.xml")?;
    let xml_truncated = xml_bytes.len() as u64 > MAX_DOCX_XML_BYTES;
    xml_bytes.truncate(MAX_DOCX_XML_BYTES as usize);
    xml.push_str(&String::from_utf8_lossy(&xml_bytes));
    let mut text = docx_xml_text(&xml);
    if text.trim().is_empty() {
        bail!("{name} has no readable text (it may contain only images)");
    }
    let (text, truncated) = ensure_text(std::mem::take(&mut text));
    let truncated = truncated || xml_truncated;
    Ok(ExtractedDocument {
        title: Some(name),
        text,
        pages: None,
        truncated,
        warnings: Vec::new(),
    })
}

fn extract_legacy_doc(path: &Path, name: String) -> Result<ExtractedDocument> {
    file_size(path, MAX_OFFICE_BYTES)?;
    let temp = TempDir::new("colm-doc")?;
    let converted = convert_legacy_doc(path, temp.path())?;
    let mut document = extract_docx(&converted, name)?;
    document.warnings.push(
        "converted the legacy .doc with the system converter; layout and images may be lost".into(),
    );
    Ok(document)
}

/// 旧 `.doc`：不自己解析二进制格式；有系统转换器就转到 DOCX，没有就明确拒绝。
fn convert_legacy_doc(path: &Path, dir: &Path) -> Result<PathBuf> {
    let output = dir.join("converted.docx");
    let mut attempts: Vec<(&str, Vec<std::ffi::OsString>)> = Vec::new();
    #[cfg(target_os = "macos")]
    {
        attempts.push((
            "/usr/bin/textutil",
            vec![
                "-convert".into(),
                "docx".into(),
                "-output".into(),
                output.clone().into_os_string(),
                path.as_os_str().to_owned(),
            ],
        ));
    }
    for program in ["soffice", "libreoffice"] {
        if which(program).is_some() {
            attempts.push((
                program,
                vec![
                    "--headless".into(),
                    "--convert-to".into(),
                    "docx".into(),
                    "--outdir".into(),
                    dir.as_os_str().to_owned(),
                    path.as_os_str().to_owned(),
                ],
            ));
        }
    }
    if attempts.is_empty() {
        bail!(
            "this computer has no .doc converter; save the file as .docx or PDF and attach it again"
        );
    }
    let mut errors = Vec::new();
    for (program, args) in attempts {
        match run_with_timeout(program, &args) {
            Ok(()) => {
                let candidate = if output.exists() {
                    output.clone()
                } else {
                    find_docx(dir).unwrap_or_default()
                };
                if candidate.exists() {
                    return Ok(candidate);
                }
                errors.push(format!("{program} produced no .docx"));
            }
            Err(error) => errors.push(format!("{program}: {error}")),
        }
    }
    bail!("cannot convert the legacy .doc ({})", errors.join("; "))
}

fn find_docx(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .find(|path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("docx"))
        })
}

fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}

fn run_with_timeout(program: &str, args: &[std::ffi::OsString]) -> Result<()> {
    let mut child = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("cannot start {program}"))?;
    let deadline = Instant::now() + Duration::from_secs(DOC_CONVERT_TIMEOUT_SECS);
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("{program} timed out after {DOC_CONVERT_TIMEOUT_SECS}s");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail = stderr.trim().lines().rev().take(3).collect::<Vec<_>>();
        bail!(
            "{program} failed: {}",
            tail.into_iter().rev().collect::<Vec<_>>().join(" / ")
        );
    }
    Ok(())
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(prefix: &str) -> Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir(&path).with_context(|| format!("cannot create {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700));
        }
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// DOCX 的 `word/document.xml` 转纯文本：保留段落/换行/制表符，去掉标签与实体。
fn docx_xml_text(xml: &str) -> String {
    let mut out = String::new();
    let mut rest = xml;
    while let Some(open) = rest.find('<') {
        let Some(close) = rest[open..].find('>') else {
            break;
        };
        let tag = &rest[open + 1..open + close];
        if (tag == "w:t" || tag.starts_with("w:t ")) && !tag.ends_with('/') {
            let start = open + close + 1;
            if let Some(end) = rest[start..].find("</w:t>") {
                out.push_str(&decode_entities(&rest[start..start + end]));
                rest = &rest[start + end + "</w:t>".len()..];
                continue;
            }
        }
        if tag == "w:tab/" || tag.starts_with("w:tab ") {
            out.push('\t');
        } else if tag == "w:br/"
            || tag == "w:cr/"
            || tag.starts_with("w:br ")
            || tag == "/w:p"
            || tag == "/w:tr"
        {
            out.push('\n');
        } else if tag == "/w:tc" {
            out.push('\t');
        }
        rest = &rest[open + close + 1..];
    }
    let mut text = String::with_capacity(out.len());
    let mut blank = 0;
    for line in out.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            blank += 1;
            if blank == 1 && !text.is_empty() {
                text.push('\n');
            }
        } else {
            blank = 0;
            text.push_str(&line);
            text.push('\n');
        }
    }
    text.trim().to_owned()
}

fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let Some(end) = tail[1..].find(';') else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let end = end + 1;
        if end > 11 {
            out.push('&');
            rest = &tail[1..];
            continue;
        }
        let entity = &tail[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &tail[end + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// 从 PDF 文字层提取文本。扫描件没有文字层，这里只能报错，不能假装读到了内容。
pub(crate) fn pdf_text(bytes: &[u8]) -> Result<(usize, String, bool)> {
    let document = pdf_extract::Document::load_mem(bytes).context("cannot parse the PDF")?;
    let pages = document.get_pages().len();
    if pages == 0 {
        bail!("the PDF has no pages");
    }
    drop(document);
    let pages_text =
        std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem_by_pages(bytes))
            .map_err(|_| anyhow::anyhow!("the PDF text extractor crashed"))?
            .context("cannot extract text from the PDF")?;
    let truncated_pages = pages_text.len() > MAX_PDF_PAGES;
    let mut text = pages_text
        .into_iter()
        .take(MAX_PDF_PAGES)
        .map(|page| page.trim().to_owned())
        .filter(|page| !page.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    if text.trim().is_empty() {
        bail!(
            "the PDF has no embedded text (it is probably a scanned image); look for its HTML page or an OCR copy"
        );
    }
    let truncated_text = text.chars().count() > MAX_PDF_TEXT_CHARS;
    if truncated_text {
        text = text.chars().take(MAX_PDF_TEXT_CHARS).collect();
    }
    Ok((pages, text, truncated_pages || truncated_text))
}

#[cfg(test)]
#[path = "document_tests.rs"]
mod document_tests;
