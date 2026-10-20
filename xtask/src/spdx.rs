// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! `cargo xtask spdx [--fix]`: every Rust source file starts with its copyright and licence
//! (REUSE-style SPDX lines), so a file copied out of the repository still says whose it is and
//! under what terms. `--fix` adds the lines to the files that lack them.

use std::path::Path;
use std::process::Command;

pub const HEADER: &str = "// SPDX-FileCopyrightText: MailCraft contributors\n// SPDX-License-Identifier: MIT OR Apache-2.0\n";

/// Whether `text` starts with the lines of [`HEADER`], whatever its line endings (a Windows
/// checkout has CRLF).
fn has_header(text: &str) -> bool {
    let mut lines = text.lines();
    HEADER.lines().all(|want| lines.next() == Some(want))
}

/// The Rust files among `files` whose text doesn't start with [`HEADER`].
pub fn missing(files: &[String], read: impl Fn(&str) -> Option<String>) -> Vec<&str> {
    files.iter().filter(|f| f.ends_with(".rs")).filter(|f| read(f).is_some_and(|text| !has_header(&text))).map(String::as_str).collect()
}

pub fn run(root: &Path, fix: bool) -> Result<(), String> {
    let out = Command::new("git")
        .current_dir(root)
        .args(["ls-files", "--cached", "--others", "--exclude-standard", "*.rs"])
        .output()
        .map_err(|e| format!("git ls-files: {e}"))?;
    let files: Vec<String> = String::from_utf8_lossy(&out.stdout).lines().map(str::to_owned).collect();
    let read = |f: &str| std::fs::read_to_string(root.join(f)).ok();
    let miss = missing(&files, read);
    if miss.is_empty() {
        println!("spdx: all {} Rust files carry their copyright and licence", files.len());
        return Ok(());
    }
    if !fix {
        return Err(format!("{} Rust file(s) lack the SPDX lines (run `cargo xtask spdx --fix`):\n  {}", miss.len(), miss.join("\n  ")));
    }
    for f in &miss {
        let path = root.join(f);
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{f}: {e}"))?;
        std::fs::write(&path, format!("{HEADER}{text}")).map_err(|e| format!("{f}: {e}"))?;
    }
    println!("spdx: added the lines to {} file(s)", miss.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_rust_files_without_the_lines() {
        let files = vec!["a.rs".to_string(), "b.rs".into(), "c.md".into(), "gone.rs".into(), "crlf.rs".into()];
        let read = |f: &str| match f {
            "a.rs" => Some(format!("{HEADER}fn a() {{}}\n")),
            "b.rs" => Some("//! b\n".into()),
            "crlf.rs" => Some(HEADER.replace('\n', "\r\n")),
            "c.md" => Some("# c\n".into()),
            _ => None,
        };
        assert_eq!(missing(&files, read), vec!["b.rs"]);
    }
}
