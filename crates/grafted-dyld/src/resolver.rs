use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct DylibResolver {
    overrides: HashMap<String, PathBuf>,
    shim_dirs: Vec<PathBuf>,
}

impl DylibResolver {
    pub fn new() -> Self {
        Self {
            overrides: HashMap::new(),
            shim_dirs: Vec::new(),
        }
    }

    pub fn add_shim_dir(&mut self, dir: PathBuf) {
        self.shim_dirs.push(dir);
    }

    pub fn add_override(&mut self, darwin_path: String, linux_path: PathBuf) {
        self.overrides.insert(darwin_path, linux_path);
    }

    /// Returns None if no mapping exists.
    pub fn resolve(&self, darwin_path: &str) -> Option<PathBuf> {
        if let Some(path) = self.overrides.get(darwin_path) {
            return Some(path.clone());
        }

        let filename = Path::new(darwin_path).file_name()?;

        // Search shim directories
        for dir in &self.shim_dirs {
            let candidate = dir.join(filename);
            if candidate.exists() {
                return Some(candidate);
            }
        }

        None
    }
}

impl Default for DylibResolver {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_override_resolution() {
        let mut resolver = DylibResolver::new();
        resolver.add_override(
            "/usr/lib/libSystem.B.dylib".into(),
            PathBuf::from("/opt/grafted/lib/libSystem.B.so"),
        );
        let result = resolver.resolve("/usr/lib/libSystem.B.dylib");
        assert_eq!(
            result,
            Some(PathBuf::from("/opt/grafted/lib/libSystem.B.so"))
        );
    }

    #[test]
    fn test_unknown_dylib() {
        let resolver = DylibResolver::new();
        assert!(resolver.resolve("/usr/lib/libFoo.dylib").is_none());
    }
}
