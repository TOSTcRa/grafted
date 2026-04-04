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

    /// Resolve a library path, substituting @rpath, @loader_path, and @executable_path.
    pub fn resolve(
        &self,
        darwin_path: &str,
        executable_path: Option<&Path>,
        loader_path: Option<&Path>,
        rpaths: &[String],
    ) -> Option<PathBuf> {
        if let Some(path) = self.overrides.get(darwin_path) {
            return Some(path.clone());
        }

        if let Some(rest) = darwin_path.strip_prefix("@executable_path/") {
            if let Some(exec_path) = executable_path {
                if let Some(exec_dir) = exec_path.parent() {
                    let candidate = exec_dir.join(rest);
                    if candidate.exists() {
                        return Some(candidate);
                    }
                }
            }
        } else if let Some(rest) = darwin_path.strip_prefix("@loader_path/") {
            if let Some(loader_p) = loader_path {
                if let Some(loader_dir) = loader_p.parent() {
                    let candidate = loader_dir.join(rest);
                    if candidate.exists() {
                        return Some(candidate);
                    }
                }
            }
        } else if let Some(rest) = darwin_path.strip_prefix("@rpath/") {
            for rpath in rpaths {
                // RPATHs themselves can contain @executable_path or @loader_path
                let rpath_resolved = self.resolve_rpath(rpath, executable_path, loader_path)?;
                let candidate = rpath_resolved.join(rest);
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }

        let filename = Path::new(darwin_path).file_name()?;

        for dir in &self.shim_dirs {
            let candidate = dir.join(filename);
            if candidate.exists() {
                return Some(candidate);
            }
        }

        let absolute = Path::new(darwin_path);
        if absolute.exists() {
            return Some(absolute.to_path_buf());
        }

        None
    }

    fn resolve_rpath(
        &self,
        rpath: &str,
        executable_path: Option<&Path>,
        loader_path: Option<&Path>,
    ) -> Option<PathBuf> {
        if let Some(rest) = rpath.strip_prefix("@executable_path/") {
            executable_path?.parent()?.join(rest).into()
        } else if rpath == "@executable_path" {
            executable_path?.parent()?.to_path_buf().into()
        } else if let Some(rest) = rpath.strip_prefix("@loader_path/") {
            loader_path?.parent()?.join(rest).into()
        } else if rpath == "@loader_path" {
            loader_path?.parent()?.to_path_buf().into()
        } else {
            Some(PathBuf::from(rpath))
        }
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
        let result = resolver.resolve("/usr/lib/libSystem.B.dylib", None, None, &[]);
        assert_eq!(
            result,
            Some(PathBuf::from("/opt/grafted/lib/libSystem.B.so"))
        );
    }

    #[test]
    fn test_unknown_dylib() {
        let resolver = DylibResolver::new();
        assert!(resolver.resolve("/usr/lib/libFoo.dylib", None, None, &[]).is_none());
    }
}
