#!/usr/bin/env python3
"""Build the shipped manual from README, generated appendices, CSS and cover.

Requires pandoc, Chrome (or CHROME_BIN), pypdf and reportlab.
Run: uv run --with pypdf --with reportlab python scripts/build-user-manual.py
"""

import json
import os
import pathlib
import re
import subprocess
import unicodedata
from pypdf import PdfReader, PdfWriter
from reportlab.pdfgen import canvas
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.cidfonts import UnicodeCIDFont
from io import BytesIO

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASE = ROOT / "docs/user-manual"
CSS = BASE / "print.css"
CHROME = os.environ.get(
    "CHROME_BIN", "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
)
TMP = ROOT / "tmp/manual-build"
VERSION = json.loads((ROOT / "gui/package.json").read_text(encoding="utf-8"))["version"]

CHAPTERS = [
    "1. 安装与启动",
    "2. 10 分钟快速开始",
    "3. 界面总览",
    "4. 项目与算例",
    "5. 新建模拟向导",
    "6. 算例五步设置",
    "7. 运行算例",
    "8. 批量运行",
    "9. 结果页",
    "10. 研究页",
    "11. 比较页（OpenBench）",
    "12. 历史页",
    "13. 资源页与远程服务器",
    "14. 设置页",
    "15. 模型版本",
    "16. 准备输入数据",
    "17. AI 助手",
    "18. 命令行 colm-cli",
    "19. 区域与流域算例（Heihe 实测）",
    "20. 配置参数总览",
    "21. 文件与目录布局",
    "22. 故障排查",
    "23. 附录",
]


def norm(t: str) -> str:
    t = unicodedata.normalize("NFKC", t)
    for a, b in [("⻚", "页"), ("⻅", "见"), ("⻆", "角"), ("⻓", "长"), ("⻔", "门")]:
        t = t.replace(a, b)
    return re.sub(r"\s+", "", t)


def build_base_md() -> str:
    s = (BASE / "README.md").read_text(encoding="utf-8")
    for f, a in [
        ("appendix-config-fields.md", "appendix-a"),
        ("appendix-tuning-parameters.md", "appendix-b"),
        ("appendix-cli.md", "appendix-c"),
    ]:
        s = s.replace(f"(appendices/{f})", f"(#{a})")
    lines = []
    for l in s.splitlines():
        if re.match(r"^## \d+\.", l):
            l = "# " + l[3:]
        elif l.startswith("### "):
            l = l[1:]
        elif l.startswith("#### "):
            l = l[1:]
        lines.append(l)
    parts = ["\n".join(lines)]
    for f, a in [
        ("appendix-config-fields.md", "appendix-a"),
        ("appendix-tuning-parameters.md", "appendix-b"),
        ("appendix-cli.md", "appendix-c"),
    ]:
        txt = (BASE / "appendices" / f).read_text(encoding="utf-8")
        txt = re.sub(
            r"^# 附录", f'<a id="{a}"></a>\n\n# 附录', txt, count=1, flags=re.M
        )
        parts.append(f'\n\n<div class="{a}">\n\n' + txt + "\n\n</div>\n")
    out = "\n".join(parts)
    (BASE / "CoLM-Desktop-用户手册.md").write_text(out, encoding="utf-8")
    return out


def strip_relative_links(md: str) -> str:
    def repl(m):
        text, target = m.group(1), m.group(2).strip()
        if target.startswith(("#", "http://", "https://", "mailto:")):
            return m.group(0)
        return text

    return re.sub(r"(?<!!)\[([^\]]+)\]\(([^)]+)\)", repl, md)


def pandoc_html(md_path: pathlib.Path, out: pathlib.Path):
    subprocess.run(
        [
            "pandoc",
            str(md_path),
            "-f",
            "gfm",
            "-t",
            "html5",
            "--standalone",
            "--embed-resources",
            "--metadata",
            "pagetitle=CoLM Desktop 用户手册",
            "--css",
            str(CSS),
            "-o",
            str(out),
        ],
        check=True,
        cwd=BASE,
    )


def chrome_pdf(html: pathlib.Path, pdf: pathlib.Path):
    subprocess.run(
        [
            CHROME,
            f"--user-data-dir={TMP / chr(99)}",
            "--headless",
            "--disable-gpu",
            "--no-pdf-header-footer",
            f"--print-to-pdf={pdf}",
            html.as_uri(),
        ],
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


def page_map(pdf: pathlib.Path, offset: int = 2):
    r = PdfReader(str(pdf))
    texts = [norm(p.extract_text() or "") for p in r.pages]
    m = {}
    for h in CHAPTERS:
        hn = norm(h)
        hits = [i for i, t in enumerate(texts) if hn in t]
        if not hits:
            raise SystemExit(f"cannot find chapter in PDF: {h}")
        m[h] = max(hits) + offset
    return m


def inject_toc(html: str, pages: dict) -> str:
    start = html.find('<h2 id="目录">')
    end = html.find("</ol>", start)
    assert start >= 0 and end > start
    block = html[start:end]
    items = list(re.finditer(r'(<li><a href="[^"]+">[\s\S]*?</a>)</li>', block))
    assert len(items) == len(CHAPTERS), (len(items), len(CHAPTERS))
    out = block
    for item, h in zip(reversed(items), reversed(CHAPTERS)):
        span = f'<span class="toc-page">第 {pages[h]} 页</span>'
        out = out[: item.end(1)] + span + out[item.end(1) :]
    return html[:start] + out + html[end:]


def stamp_and_merge(body_pdf: pathlib.Path, cover_pdf: pathlib.Path, out: pathlib.Path):
    pdfmetrics.registerFont(UnicodeCIDFont("STSong-Light"))
    body = PdfReader(str(body_pdf))
    n = len(body.pages)
    total = n + 1
    buf = BytesIO()
    c = canvas.Canvas(buf, pagesize=(595.276, 841.89))
    for i in range(1, n + 1):
        width, height = map(float, body.pages[i - 1].mediabox[2:])
        c.setPageSize((width, height))
        c.setFont("STSong-Light", 8)
        c.setFillColorRGB(0.42, 0.47, 0.46)
        c.drawString(42.6, 20, f"CoLM Desktop 用户手册  v{VERSION}")
        c.drawRightString(width - 42.6, 20, f"第 {i + 1} 页 / 共 {total} 页")
        c.showPage()
    c.save()
    buf.seek(0)
    overlay = PdfReader(buf)
    w = PdfWriter()
    for i, p in enumerate(body.pages):
        p.merge_page(overlay.pages[i])
        w.add_page(p)
    cover = PdfReader(str(cover_pdf))
    final = PdfWriter()
    for p in list(cover.pages) + list(w.pages):
        final.add_page(p)
    final.add_metadata(
        {"/Title": f"CoLM Desktop 用户手册 v{VERSION}", "/Author": "CoLM Desktop"}
    )
    with open(out, "wb") as f:
        final.write(f)
    return len(final.pages)


def main():
    TMP.mkdir(parents=True, exist_ok=True)
    base_md_text = build_base_md()
    (TMP / "pdf.md").write_text(strip_relative_links(base_md_text), encoding="utf-8")
    pandoc_html(TMP / "pdf.md", BASE / ".body-pdf.html")
    chrome_pdf(BASE / ".body-pdf.html", TMP / "body-base.pdf")
    pages = page_map(TMP / "body-base.pdf")
    print("page map", pages)
    html = (BASE / ".body-pdf.html").read_text(encoding="utf-8")
    html = inject_toc(html, pages)
    html = html.replace(
        "</style>",
        ".toc-page{float:right;color:#6a7775;font-size:8.6pt;font-weight:400;}\n</style>",
    )
    (BASE / ".body-pdf-numbered.html").write_text(html, encoding="utf-8")
    chrome_pdf(BASE / ".body-pdf-numbered.html", TMP / "body-numbered.pdf")
    pages2 = page_map(TMP / "body-numbered.pdf")
    print("page map after toc", pages2)
    if pages2 != pages:
        html2 = inject_toc(
            (BASE / ".body-pdf.html").read_text(encoding="utf-8"), pages2
        )
        html2 = html2.replace(
            "</style>",
            ".toc-page{float:right;color:#6a7775;font-size:8.6pt;font-weight:400;}\n</style>",
        )
        (BASE / ".body-pdf-numbered.html").write_text(html2, encoding="utf-8")
        chrome_pdf(BASE / ".body-pdf-numbered.html", TMP / "body-numbered2.pdf")
        pages3 = page_map(TMP / "body-numbered2.pdf")
        print("page map after second toc", pages3)
        body_pdf = TMP / "body-numbered2.pdf"
    else:
        body_pdf = TMP / "body-numbered.pdf"
    # 主 HTML：干净目录、内部附录锚点、旧版封面
    pandoc_html(BASE / "CoLM-Desktop-用户手册.md", BASE / ".body-base.html")
    html = (BASE / ".body-base.html").read_text(encoding="utf-8")
    cover_html = (BASE / "cover.html").read_text(encoding="utf-8")
    cover_body = re.search(r"<body[^>]*>([\s\S]*?)</body>", cover_html).group(1)
    cover_css = "\n".join(re.findall(r"<style[^>]*>([\s\S]*?)</style>", cover_html))
    cover_css = "\n".join(
        l for l in cover_css.splitlines() if not l.strip().startswith("@page")
    )
    cover_css += "\n.cover{height:258mm;}\n.cover img{border:none;margin:0;box-shadow:none;}\n@media screen{body{background:#fff;}}"
    idx = html.find("</head>")
    html = html[:idx] + f"<style>{cover_css}</style>" + html[idx:]
    m = re.search(r"<body[^>]*>", html)
    html = html[: m.end()] + "\n" + cover_body + "\n" + html[m.end() :]
    (BASE / "CoLM-Desktop-用户手册.html").write_text(html, encoding="utf-8")
    cover_pdf = TMP / "cover.pdf"
    chrome_pdf(BASE / "cover.html", cover_pdf)
    total = stamp_and_merge(body_pdf, cover_pdf, BASE / "CoLM-Desktop-用户手册.pdf")
    print("final pages", total)
    for f in [".body-pdf.html", ".body-pdf-numbered.html", ".body-base.html"]:
        (BASE / f).unlink(missing_ok=True)


if __name__ == "__main__":
    main()
