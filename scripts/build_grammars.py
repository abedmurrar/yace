#!/usr/bin/env python3
"""Build the `yace-grammars` package (hybrid grammars, TODO.md D29).

Release builds of YACE compile in the grammars people use most; these
grammars ship separately as an installable data plugin instead:
C, C++, Go, Java, Ruby, SCSS, SQL and Dockerfile. This script compiles each
grammar's C sources (from the same crates.io crates the `all-grammars`
cargo feature uses, so versions match) into a shared library, copies the
queries, writes the manifest and licences, and packs everything:

    scripts/build_grammars.py [OUT_DIR]        (default: dist/)
    -> OUT_DIR/yace-grammars/                   (plugin folder)
    -> OUT_DIR/yace-grammars-<version>-<arch>.tar.gz

Install with "Extensions: Install from Package…" or
`yace --install-plugin OUT_DIR/yace-grammars-….tar.gz`.
Needs a C compiler (`cc`). Tests use YACE_GRAMMARS_ONLY=cpp,go to build a
subset and YACE_GRAMMARS_OPT=-O0 to build faster.
"""
import json, os, platform, shutil, subprocess, sys, tarfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, "dist"))
CC = os.environ.get("CC", "cc")

# language id -> (crate, exported symbol, [highlight queries as (crate, file)], [injection queries])
GRAMMARS = {
    "c": ("tree-sitter-c", "tree_sitter_c", [("tree-sitter-c", "highlights.scm")], []),
    "cpp": ("tree-sitter-cpp", "tree_sitter_cpp",
            [("tree-sitter-c", "highlights.scm"), ("tree-sitter-cpp", "highlights.scm")], []),
    "go": ("tree-sitter-go", "tree_sitter_go", [("tree-sitter-go", "highlights.scm")], []),
    "java": ("tree-sitter-java", "tree_sitter_java", [("tree-sitter-java", "highlights.scm")], []),
    "ruby": ("tree-sitter-ruby", "tree_sitter_ruby", [("tree-sitter-ruby", "highlights.scm")], []),
    "scss": ("tree-sitter-scss", "tree_sitter_scss",
             [("tree-sitter-css", "highlights.scm"), ("tree-sitter-scss", "highlights.scm")], []),
    "sql": ("tree-sitter-sequel", "tree_sitter_sql", [("tree-sitter-sequel", "highlights.scm")], []),
    "dockerfile": ("tree-sitter-containerfile", "tree_sitter_containerfile",
                   [("tree-sitter-containerfile", "highlights.scm")],
                   [("tree-sitter-containerfile", "injections.scm")]),
}

MIT = """MIT License

Copyright (c) {holders}

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE."""


def main():
    meta = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--format-version", "1", "--features", "yace-syntax/all-grammars"], cwd=ROOT))
    pkgs = {}
    for p in meta["packages"]:
        pkgs.setdefault(p["name"], p)
    version = next(p["version"] for p in meta["packages"] if p["name"] == "yace")
    crate_dir = lambda name: os.path.dirname(pkgs[name]["manifest_path"])

    dest = os.path.join(OUT, "yace-grammars")
    shutil.rmtree(dest, ignore_errors=True)
    os.makedirs(os.path.join(dest, "grammars"))
    manifest = [
        "# Tree-sitter grammars for YACE, built by scripts/build_grammars.py.",
        'id = "dev.yace.grammars"',
        'name = "YACE Grammars"',
        f'version = "{version}"',
        'description = "Syntax highlighting for C, C++, Go, Java, Ruby, SCSS, SQL and Dockerfile."',
        'publisher = "YACE"',
        'license = "MIT (grammars: see LICENSES.md)"',
        f'yace = ">={version.split("-")[0]}"',
        f'os = ["{platform.system().lower()}"]',
        f'arch = ["{platform.machine()}"]',
        "",
    ]
    licences = ["# Licences of the grammars in this package", ""]
    used = set()
    only = [x for x in os.environ.get("YACE_GRAMMARS_ONLY", "").split(",") if x]
    opt = os.environ.get("YACE_GRAMMARS_OPT", "-O2")
    for lang, (crate, symbol, hl, inj) in GRAMMARS.items():
        if only and lang not in only:
            continue
        src = os.path.join(crate_dir(crate), "src")
        files = [os.path.join(src, f) for f in ("parser.c", "scanner.c") if os.path.exists(os.path.join(src, f))]
        lib = os.path.join("grammars", f"{lang}.so")
        cmd = [CC, "-shared", "-fPIC", opt, "-std=c11", "-I", src, *files, "-o", os.path.join(dest, lib)]
        print("  cc", lang, flush=True)
        subprocess.check_call(cmd)
        qdir = os.path.join("queries", lang)
        os.makedirs(os.path.join(dest, qdir), exist_ok=True)
        rel = lambda qs: [copy_query(dest, qdir, c, f, crate_dir) for c, f in qs]
        manifest += [
            "[[grammars]]",
            f'language = "{lang}"',
            f'library = "{lib}"',
            f'symbol = "{symbol}"',
            "highlights = " + json.dumps(rel(hl)),
            "injections = " + json.dumps(rel(inj)),
            "",
        ]
        for c in {crate, *[c for c, _ in hl + inj]}:
            if c in used:
                continue
            used.add(c)
            p = pkgs[c]
            d = crate_dir(c)
            lic_files = [f for f in sorted(os.listdir(d)) if f.upper().startswith(("LICENSE", "LICENCE", "COPYING"))]
            text = open(os.path.join(d, lic_files[0])).read().strip() if lic_files else MIT.format(
                holders=", ".join(a.split(" <")[0] for a in p.get("authors") or []) or f"the {c} authors")
            licences += [f"## {c} {p['version']} ({p.get('license') or 'see text'})",
                         p.get("repository") or "", "", "```text", text, "```", ""]
    open(os.path.join(dest, "yace-plugin.toml"), "w").write("\n".join(manifest))
    open(os.path.join(dest, "LICENSES.md"), "w").write("\n".join(licences))

    pkg = os.path.join(OUT, f"yace-grammars-{version}-{platform.machine()}.tar.gz")
    with tarfile.open(pkg, "w:gz") as tar:
        for base, _, names in sorted(os.walk(dest)):
            for n in sorted(names):
                full = os.path.join(base, n)
                info = tar.gettarinfo(full, os.path.relpath(full, dest))
                info.uid = info.gid = 0
                info.uname = info.gname = ""
                info.mtime = 0
                with open(full, "rb") as f:
                    tar.addfile(info, f)
    size = sum(os.path.getsize(os.path.join(b, n)) for b, _, ns in os.walk(dest) for n in ns)
    print(f"{dest} ({size / 1e6:.1f} MB)\n{pkg} ({os.path.getsize(pkg) / 1e6:.1f} MB)")


def copy_query(dest, qdir, crate, fname, crate_dir):
    name = f"{crate.replace('tree-sitter-', '')}-{fname}"
    shutil.copy(os.path.join(crate_dir(crate), "queries", fname), os.path.join(dest, qdir, name))
    return f"{qdir}/{name}"


if __name__ == "__main__":
    main()
