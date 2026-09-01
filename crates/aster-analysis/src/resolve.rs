use std::path::PathBuf;

/// Resolves Lua module names (`a.b.c`) to files using `package.path`-style
/// patterns with `?` placeholders, relative to a project root.
pub struct Resolver {
    root: PathBuf,
    search_path: Vec<String>,
}

impl Resolver {
    pub fn new(root: PathBuf, search_path: Vec<String>) -> Self {
        Self { root, search_path }
    }

    /// Resolve `module` to a file path relative to `root`, or `None` if no
    /// search-path pattern matches an existing file.
    pub fn resolve(&self, module: &str) -> Option<PathBuf> {
        let module_path = module.replace('.', "/");
        for pattern in &self.search_path {
            let candidate = pattern.replace('?', &module_path);
            let candidate = candidate.strip_prefix("./").unwrap_or(&candidate);
            if self.root.join(candidate).is_file() {
                return Some(PathBuf::from(candidate));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolver() -> Resolver {
        Resolver::new(
            PathBuf::from("tests/fixtures/clean"),
            vec!["./?.lua".to_string(), "./?/init.lua".to_string()],
        )
    }

    #[test]
    fn resolves_top_level_module() {
        assert_eq!(
            resolver().resolve("player"),
            Some(PathBuf::from("player.lua"))
        );
    }

    #[test]
    fn resolves_nested_module_with_dots() {
        assert_eq!(
            resolver().resolve("lib.util"),
            Some(PathBuf::from("lib/util.lua"))
        );
    }

    #[test]
    fn resolves_init_lua_pattern() {
        assert_eq!(
            resolver().resolve("pkg"),
            Some(PathBuf::from("pkg/init.lua"))
        );
    }

    #[test]
    fn returns_none_for_unknown_module() {
        assert_eq!(resolver().resolve("ghost"), None);
    }
}
