use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

/// Collect all `.lua` files under `root`, respecting `.gitignore` rules
/// (applied even when `root` is not inside a git repository).
/// Returns paths relative to `root`, sorted for determinism.
pub fn discover_lua_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let walker = WalkBuilder::new(root).require_git(false).build();
    for entry in walker.flatten() {
        let path = entry.path();
        if path.is_file()
            && path.extension().is_some_and(|ext| ext == "lua")
            && let Ok(relative) = path.strip_prefix(root)
        {
            files.push(relative.to_path_buf());
        }
    }
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_lua_files_respecting_gitignore() {
        // Built at runtime: an on-disk fixture can't work because the fixture's
        // own .gitignore would prevent git from ever tracking the ignored file.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("main.lua"), "return {}\n").unwrap();
        std::fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
        std::fs::create_dir(root.join("ignored")).unwrap();
        std::fs::write(root.join("ignored").join("skip.lua"), "return {}\n").unwrap();

        let files = discover_lua_files(root);
        assert_eq!(files, vec![PathBuf::from("main.lua")]);
    }
}
