use std::path::Path;

pub const NERD_FONT: bool = false;

pub const DIR_OPEN: &str = "▾";
pub const DIR_CLOSED: &str = "▸";

pub fn file(path: &Path) -> &'static str {
    if !NERD_FONT {
        return " ";
    }

    match extension(path) {
        "rs" => "",
        "ts" | "tsx" => "",
        "js" | "jsx" | "mjs" | "cjs" => "",
        "py" => "",
        "go" => "",
        "zig" => "",
        "c" | "h" => "",
        "cpp" | "hpp" | "cc" => "",
        "json" | "jsonc" => "",
        "toml" => "",
        "yaml" | "yml" => "",
        "md" | "mdx" => "",
        "html" => "",
        "css" | "scss" | "sass" => "",
        "sh" | "bash" | "zsh" | "fish" => "",
        "lock" => "",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "ico" => "",
        "zip" | "tar" | "gz" | "xz" | "zst" => "",
        _ => "",
    }
}

pub fn directory(expanded: bool) -> &'static str {
    if expanded {
        DIR_OPEN
    } else {
        DIR_CLOSED
    }
}

fn extension(path: &Path) -> &str {
    path.extension().and_then(|ext| ext.to_str()).unwrap_or("")
}
