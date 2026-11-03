//! File-type icons (spec §8.6): Nerd Font devicons when
//! `workbench.glyphs = "nerdFont"`, a coloured dot in Unicode mode, none in
//! ASCII mode. Lookups are `match`es on short lower-cased keys built on the
//! stack, so drawing a row never allocates.

use ratatui::style::Color;
use yace_config::Glyphs;

/// An icon: the glyph (Nerd Font) and the file type's colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Icon {
    pub glyph: &'static str,
    pub color: Color,
}

const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

const fn icon(glyph: &'static str, hex: u32) -> Icon {
    Icon { glyph, color: rgb(hex) }
}

/// The dot drawn for a known file type in Unicode mode.
pub const DOT: &str = "●";

// Nerd Fonts v3 codepoints (devicons, seti, Font Awesome, Linux logos).
const RUST: Icon = icon("\u{e7a8}", 0xdea584);
const TS: Icon = icon("\u{e628}", 0x519aba);
const REACT: Icon = icon("\u{e7ba}", 0x20c2e3);
const JS: Icon = icon("\u{e74e}", 0xcbcb41);
const JSON: Icon = icon("\u{e60b}", 0xcbcb41);
const MARKDOWN: Icon = icon("\u{e73e}", 0x519aba);
const PYTHON: Icon = icon("\u{e73c}", 0xffbc03);
const GO: Icon = icon("\u{e627}", 0x00add8);
const HTML: Icon = icon("\u{e736}", 0xe44d26);
const CSS: Icon = icon("\u{e749}", 0x42a5f5);
const SASS: Icon = icon("\u{e603}", 0xf55385);
const SHELL: Icon = icon("\u{e795}", 0x89e051);
const YAML: Icon = icon("\u{e6a8}", 0x6d8086);
const TOML: Icon = icon("\u{e6b2}", 0x9c4221);
const LOCK: Icon = icon("\u{f023}", 0xbbbbbb);
const JAVA: Icon = icon("\u{e738}", 0xcc3e44);
const RUBY: Icon = icon("\u{e791}", 0xcc342d);
const C: Icon = icon("\u{e61e}", 0x599eff);
const CPP: Icon = icon("\u{e61d}", 0xf34b7d);
const HEADER: Icon = icon("\u{f0fd}", 0xa074c4);
const PHP: Icon = icon("\u{e73d}", 0xa074c4);
const SQL: Icon = icon("\u{e706}", 0xdad8d8);
const IMAGE: Icon = icon("\u{f1c5}", 0xa074c4);
const PDF: Icon = icon("\u{f1c1}", 0xb30b00);
const ARCHIVE: Icon = icon("\u{f1c6}", 0xeca517);
const VUE: Icon = icon("\u{e6a0}", 0x8dc149);
const SVELTE: Icon = icon("\u{e697}", 0xff3e00);
const LUA: Icon = icon("\u{e620}", 0x51a0cf);
const KOTLIN: Icon = icon("\u{e634}", 0x7f52ff);
const SWIFT: Icon = icon("\u{e755}", 0xe37933);
const DART: Icon = icon("\u{e798}", 0x03589c);
const CSHARP: Icon = icon("\u{f031b}", 0x9b4f96);
const TEXT: Icon = icon("\u{f15c}", 0x89e051);
const CODE: Icon = icon("\u{f121}", 0xe37933);
const TABLE: Icon = icon("\u{f1c3}", 0x89e051);
const KEY: Icon = icon("\u{f084}", 0xfaf743);
const DOCKER: Icon = icon("\u{f308}", 0x458ee6);
const GIT: Icon = icon("\u{e702}", 0xf54d27);
const NPM: Icon = icon("\u{e71e}", 0xe8274b);
const GNU: Icon = icon("\u{e779}", 0x6d8086);
const BOOK: Icon = icon("\u{f02d}", 0xd0bf41);
const VSCODE: Icon = icon("\u{e70c}", 0x007acc);
const PRISMA: Icon = icon("\u{e684}", 0x5a67d8);
const GRAPHQL: Icon = icon("\u{e662}", 0xe535ab);
const NIX: Icon = icon("\u{f313}", 0x7ebae4);
const ZIG: Icon = icon("\u{e6a9}", 0xf69a1b);
const ELIXIR: Icon = icon("\u{e62d}", 0xa074c4);
const HASKELL: Icon = icon("\u{e777}", 0xa074c4);
const SCALA: Icon = icon("\u{e737}", 0xcc3e44);
const R: Icon = icon("\u{f25d}", 0x358a5b);
const LOG: Icon = icon("\u{f18d}", 0xdddddd);
const DIFF: Icon = icon("\u{f440}", 0x41535b);
const FONT: Icon = icon("\u{f031}", 0xececec);
const AUDIO: Icon = icon("\u{f1c7}", 0x66d8ef);
const VIDEO: Icon = icon("\u{f1c8}", 0xfd971f);
const TERRAFORM: Icon = icon("\u{e69a}", 0x5f43e9);
const PROTO: Icon = icon("\u{f0ad}", 0xd0bf41);

const FOLDER_GIT: Icon = icon("\u{e5fb}", 0xf54d27);
const FOLDER_NPM: Icon = icon("\u{e5fa}", 0xcc3e44);
const FOLDER_GITHUB: Icon = icon("\u{e5fd}", 0xbbbbbb);
const FOLDER_VSCODE: Icon = VSCODE;

/// Lower-case `s` into `buf` (ASCII only; longer or non-ASCII names give
/// `None`, which never matches a known type).
fn lower<'a>(s: &str, buf: &'a mut [u8; 32]) -> Option<&'a str> {
    let b = s.as_bytes();
    if b.len() > buf.len() || !s.is_ascii() {
        return None;
    }
    for (o, c) in buf.iter_mut().zip(b) {
        *o = c.to_ascii_lowercase();
    }
    std::str::from_utf8(&buf[..b.len()]).ok()
}

fn by_name(name: &str) -> Option<Icon> {
    let mut buf = [0u8; 32];
    Some(match lower(name, &mut buf)? {
        "package.json" | "package-lock.json" | ".npmrc" | ".npmignore" => NPM,
        "cargo.toml" | "cargo.lock" | "rust-toolchain" | "rust-toolchain.toml" => RUST,
        "dockerfile"
        | "containerfile"
        | ".dockerignore"
        | "docker-compose.yml"
        | "docker-compose.yaml"
        | "compose.yml"
        | "compose.yaml" => DOCKER,
        ".gitignore" | ".gitattributes" | ".gitmodules" | ".gitkeep" | ".git-blame-ignore-revs" => GIT,
        "makefile" | "gnumakefile" | "cmakelists.txt" => GNU,
        "license" | "license.md" | "license.txt" | "licence" | "copying" | "license-mit" | "license-apache" => BOOK,
        "tsconfig.json" | "jsconfig.json" => TS,
        ".env" | ".env.local" | ".env.example" | ".env.development" | ".env.production" | ".env.test" => KEY,
        "yarn.lock" | "pnpm-lock.yaml" | "bun.lockb" | "flake.lock" | "poetry.lock" | "gemfile.lock" => LOCK,
        "gemfile" | "rakefile" => RUBY,
        "go.mod" | "go.sum" => GO,
        ".bashrc" | ".zshrc" | ".profile" | ".bash_profile" => SHELL,
        ".editorconfig" | ".prettierrc" | ".eslintrc" => CODE,
        _ => return None,
    })
}

fn by_extension(ext: &str) -> Option<Icon> {
    let mut buf = [0u8; 32];
    Some(match lower(ext, &mut buf)? {
        "rs" => RUST,
        "ts" | "mts" | "cts" => TS,
        "tsx" | "jsx" => REACT,
        "js" | "mjs" | "cjs" => JS,
        "json" | "jsonc" | "json5" => JSON,
        "md" | "mdx" | "markdown" => MARKDOWN,
        "py" | "pyi" | "pyw" => PYTHON,
        "go" => GO,
        "html" | "htm" => HTML,
        "css" => CSS,
        "scss" | "sass" | "less" => SASS,
        "sh" | "bash" | "zsh" | "fish" | "ps1" => SHELL,
        "yml" | "yaml" => YAML,
        "toml" | "ini" | "cfg" | "conf" => TOML,
        "lock" => LOCK,
        "java" | "jar" | "gradle" => JAVA,
        "rb" | "erb" => RUBY,
        "c" => C,
        "cpp" | "cc" | "cxx" | "c++" => CPP,
        "h" | "hpp" | "hh" | "hxx" => HEADER,
        "php" => PHP,
        "sql" | "sqlite" | "db" => SQL,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "svg" | "avif" | "tiff" => IMAGE,
        "pdf" => PDF,
        "zip" | "gz" | "tgz" | "tar" | "xz" | "bz2" | "7z" | "rar" | "zst" => ARCHIVE,
        "vue" => VUE,
        "svelte" => SVELTE,
        "lua" => LUA,
        "kt" | "kts" => KOTLIN,
        "swift" => SWIFT,
        "dart" => DART,
        "cs" | "csx" => CSHARP,
        "txt" | "rst" => TEXT,
        "xml" | "xsd" | "xsl" | "plist" => CODE,
        "csv" | "tsv" | "xlsx" | "xls" => TABLE,
        "env" | "pem" | "key" | "crt" => KEY,
        "prisma" => PRISMA,
        "graphql" | "gql" => GRAPHQL,
        "nix" => NIX,
        "zig" => ZIG,
        "ex" | "exs" => ELIXIR,
        "hs" => HASKELL,
        "scala" | "sc" => SCALA,
        "r" => R,
        "log" => LOG,
        "diff" | "patch" => DIFF,
        "ttf" | "otf" | "woff" | "woff2" => FONT,
        "mp3" | "wav" | "flac" | "ogg" => AUDIO,
        "mp4" | "mkv" | "mov" | "webm" => VIDEO,
        "tf" | "tfvars" => TERRAFORM,
        "proto" => PROTO,
        "dockerfile" => DOCKER,
        _ => return None,
    })
}

/// The type icon of a file name, if YACE knows the type.
pub fn file_type(name: &str) -> Option<Icon> {
    if let Some(i) = by_name(name) {
        return Some(i);
    }
    if name.starts_with(".env.") {
        return Some(KEY);
    }
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    by_extension(ext)
}

/// A special folder's icon (Nerd Font mode).
pub fn folder_type(name: &str) -> Option<Icon> {
    Some(match name {
        ".git" => FOLDER_GIT,
        "node_modules" => FOLDER_NPM,
        ".github" => FOLDER_GITHUB,
        ".vscode" => FOLDER_VSCODE,
        _ => return None,
    })
}

/// What to draw before a file name: the Nerd Font glyph, or the Unicode
/// dot, with the type's colour. `None` in ASCII mode, when icons are off,
/// or (Unicode) for unknown types.
pub fn for_file(name: &str, glyphs: Glyphs, enabled: bool) -> Option<Icon> {
    if !enabled {
        return None;
    }
    match glyphs {
        Glyphs::Ascii => None,
        Glyphs::Unicode => file_type(name).map(|i| Icon { glyph: DOT, color: i.color }),
        Glyphs::NerdFont => file_type(name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookups() {
        assert_eq!(file_type("main.rs"), Some(RUST));
        assert_eq!(file_type("Cargo.toml"), Some(RUST), "names win over extensions");
        assert_eq!(file_type("PACKAGE.JSON"), Some(NPM), "case-insensitive");
        assert_eq!(file_type("App.TSX"), Some(REACT));
        assert_eq!(file_type("archive.tar.gz"), Some(ARCHIVE));
        assert_eq!(file_type("Dockerfile"), Some(DOCKER));
        assert_eq!(file_type(".env.staging"), Some(KEY));
        assert_eq!(file_type(".bashrc"), Some(SHELL));
        assert_eq!(file_type(".hidden"), None, "a dotfile has no extension");
        assert_eq!(file_type("noext"), None);
        assert_eq!(file_type("weird.unknownext"), None);
        assert_eq!(file_type("ünïcode.rs"), Some(RUST));
        assert_eq!(file_type(&format!("{}.rs", "x".repeat(300))), Some(RUST));
        assert_eq!(folder_type("node_modules"), Some(FOLDER_NPM));
        assert_eq!(for_file("a.rs", Glyphs::Unicode, true).map(|i| i.glyph), Some(DOT));
        assert_eq!(for_file("a.rs", Glyphs::Ascii, true), None);
        assert_eq!(for_file("a.rs", Glyphs::NerdFont, false), None);
        assert_eq!(for_file("a.unknown", Glyphs::Unicode, true), None);
    }

    /// Every glyph is one cell wide, so rows stay aligned.
    #[test]
    fn glyphs_are_one_cell() {
        use unicode_width::UnicodeWidthStr;
        for name in [
            "a.rs",
            "a.ts",
            "a.tsx",
            "a.js",
            "a.json",
            "a.md",
            "a.py",
            "a.go",
            "a.html",
            "a.css",
            "a.scss",
            "a.sh",
            "a.yml",
            "a.toml",
            "a.lock",
            "a.java",
            "a.rb",
            "a.c",
            "a.cpp",
            "a.h",
            "a.php",
            "a.sql",
            "a.png",
            "a.pdf",
            "a.zip",
            "a.vue",
            "a.svelte",
            "a.lua",
            "a.kt",
            "a.swift",
            "a.dart",
            "a.cs",
            "a.txt",
            "a.xml",
            "a.csv",
            "a.pem",
            "Dockerfile",
            ".gitignore",
            "package.json",
            "Makefile",
            "LICENSE",
            "a.prisma",
            "a.graphql",
            "a.nix",
            "a.zig",
            "a.ex",
            "a.hs",
            "a.scala",
            "a.r",
            "a.log",
            "a.diff",
            "a.ttf",
            "a.mp3",
            "a.mp4",
            "a.tf",
            "a.proto",
        ] {
            let i = file_type(name).unwrap_or_else(|| panic!("{name}"));
            assert_eq!(UnicodeWidthStr::width(i.glyph), 1, "{name}");
        }
    }
}
