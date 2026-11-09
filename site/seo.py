#!/usr/bin/env python3
"""Search engine tags for the built documentation site (mdBook can't set
them per page).

    python3 site/seo.py site/book https://abedmurrar.github.io/yace/

For every page of the built book (site/book), in place:
- a description from the page's first paragraph (the site description on
  the home page), replacing mdBook's one shared description;
- the canonical URL, Open Graph and Twitter tags, and the share image;
- on the home page, a keyword-rich title and schema.org structured data.
print.html (every page in one) and toc.html are marked noindex, so they
don't compete with the real pages. Writes sitemap.xml, with each page's
last change from git (the workflow checks out the full history).
Standard library only.
"""
import html
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "site" / "src"
NAME = "YACE"
TAGLINE = "a fast, light terminal code editor with VS Code keys"
DESCRIPTION = (
    "YACE is a fast, light code editor for the terminal with VS Code's keys: "
    "multi-root workspaces, fuzzy Quick Open, search and replace, an integrated "
    "terminal, language servers, Git and plugins. About 9 MiB of memory at idle."
)
KEYWORDS = (
    "terminal code editor, tui editor, terminal ide, rust code editor, vs code alternative, "
    "vscode keybindings terminal, multi-root workspace, code-workspace, lsp editor, "
    "lightweight code editor, linux code editor, ratatui, yace"
)
NOINDEX = {"print.html", "toc.html", "404.html"}


def version() -> str:
    text = (ROOT / "Cargo.toml").read_text()
    m = re.search(r'\[workspace\.package\][^\[]*?\nversion = "([^"]+)"', text, re.S)
    return m.group(1) if m else ""


def first_paragraph(page: str) -> str | None:
    """Plain text of the first real paragraph in the page's <main>."""
    main = re.search(r"<main>(.*?)</main>", page, re.S)
    if not main:
        return None
    for p in re.findall(r"<p>(.*?)</p>", main.group(1), re.S):
        text = html.unescape(re.sub(r"<[^>]+>", "", p))
        text = re.sub(r"\s+", " ", text).strip()
        if len(text) >= 40:
            return text
    return None


def shorten(text: str, limit: int = 160) -> str:
    if len(text) <= limit:
        return text
    return text[: limit - 3].rsplit(" ", 1)[0].rstrip(",.;:") + "..."


def source_files(rel: str) -> list[Path]:
    """The Markdown files a page is built from (its page and its includes)."""
    md = SRC / (rel[: -len(".html")] + ".md")
    if rel == "index.html" and not md.exists():
        md = SRC / "introduction.md"
    if not md.exists():
        return []
    files = [md]
    for inc in re.findall(r"\{\{#include\s+([^}\s:]+)", md.read_text()):
        p = (md.parent / inc).resolve()
        if p.exists():
            files.append(p)
    return files


def lastmod(files: list[Path]) -> str | None:
    dates = []
    for f in files:
        out = subprocess.run(
            ["git", "log", "-1", "--format=%cs", "--", str(f)], cwd=ROOT, capture_output=True, text=True
        ).stdout.strip()
        if out:
            dates.append(out)
    return max(dates) if dates else None


def structured_data(site: str) -> dict:
    return {
        "@context": "https://schema.org",
        "@graph": [
            {"@type": "WebSite", "name": NAME, "url": site, "description": DESCRIPTION},
            {
                "@type": "SoftwareApplication",
                "name": NAME,
                "alternateName": "Yet Another Code Editor",
                "description": DESCRIPTION,
                "url": site,
                "applicationCategory": "DeveloperApplication",
                "applicationSubCategory": "Code editor",
                "operatingSystem": "Linux",
                "softwareVersion": version(),
                "license": "https://spdx.org/licenses/MIT.html",
                "downloadUrl": "https://github.com/abedmurrar/yace/releases/latest",
                "installUrl": site + "installation.html",
                "image": site + "og.png",
                "author": {"@type": "Person", "name": "Abed Murrar", "url": "https://github.com/abedmurrar"},
                "offers": {"@type": "Offer", "price": "0", "priceCurrency": "USD"},
            },
            {
                "@type": "SoftwareSourceCode",
                "name": NAME,
                "codeRepository": "https://github.com/abedmurrar/yace",
                "programmingLanguage": "Rust",
                "license": "https://spdx.org/licenses/MIT.html",
            },
        ],
    }


def main() -> None:
    book = Path(sys.argv[1])
    site = sys.argv[2].rstrip("/") + "/"
    image = site + "og.png"
    sitemap = []
    for path in sorted(book.rglob("*.html")):
        rel = path.relative_to(book).as_posix()
        if rel.startswith(("apt/", "fonts/", "css/", "FontAwesome/")) or rel.startswith("google"):
            continue
        page = path.read_text()
        if rel in NOINDEX:
            page = page.replace("<head>", '<head>\n        <meta name="robots" content="noindex">', 1)
            path.write_text(page)
            continue
        # mdBook writes the first chapter twice (index.html and
        # introduction.html): both point at the home page.
        duplicate = rel == "introduction.html"
        url = site if rel == "index.html" or duplicate else site + rel
        home = rel == "index.html"
        title_m = re.search(r"<title>(.*?)</title>", page, re.S)
        title = html.unescape(title_m.group(1)).strip() if title_m else NAME
        if home:
            title = f"{NAME} — {TAGLINE}"
            page = re.sub(r"<title>.*?</title>", f"<title>{html.escape(title)}</title>", page, count=1, flags=re.S)
        para = first_paragraph(page)
        if home or not para:
            desc = DESCRIPTION if home else f"{title}: documentation for {NAME}, {TAGLINE}."
        elif len(para) < 80:
            desc = f"{para} ({title}, {NAME} documentation.)"
        else:
            desc = para
        desc = shorten(desc)
        a = lambda s: html.escape(s, quote=True)  # noqa: E731
        tags = [
            f'<meta name="description" content="{a(desc)}">',
            f'<link rel="canonical" href="{a(url)}">',
            '<meta name="author" content="Abed Murrar">',
            f'<meta name="keywords" content="{a(KEYWORDS)}">',
            f'<meta property="og:site_name" content="{NAME}">',
            '<meta property="og:type" content="website">',
            '<meta property="og:locale" content="en_US">',
            f'<meta property="og:url" content="{a(url)}">',
            f'<meta property="og:title" content="{a(title)}">',
            f'<meta property="og:description" content="{a(desc)}">',
            f'<meta property="og:image" content="{a(image)}">',
            '<meta property="og:image:width" content="1200">',
            '<meta property="og:image:height" content="630">',
            f'<meta property="og:image:alt" content="{a(NAME + ": " + TAGLINE)}">',
            '<meta name="twitter:card" content="summary_large_image">',
            f'<meta name="twitter:title" content="{a(title)}">',
            f'<meta name="twitter:description" content="{a(desc)}">',
            f'<meta name="twitter:image" content="{a(image)}">',
        ]
        if home:
            tags.append(
                '<script type="application/ld+json">' + json.dumps(structured_data(site), ensure_ascii=False) + "</script>"
            )
        block = "\n        ".join(tags)
        page, n = re.subn(r'<meta name="description" content="[^"]*">', block, page, count=1)
        if n == 0:
            page = page.replace("</head>", f"        {block}\n    </head>", 1)
        path.write_text(page)
        if duplicate:
            continue
        entry = f"  <url><loc>{html.escape(url)}</loc>"
        mod = lastmod(source_files(rel))
        if mod:
            entry += f"<lastmod>{mod}</lastmod>"
        sitemap.append(entry + "</url>")
    (book / "sitemap.xml").write_text(
        '<?xml version="1.0" encoding="UTF-8"?>\n'
        '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n' + "\n".join(sitemap) + "\n</urlset>\n"
    )
    print(f"SEO tags on {len(sitemap)} pages; sitemap.xml written")


if __name__ == "__main__":
    main()
