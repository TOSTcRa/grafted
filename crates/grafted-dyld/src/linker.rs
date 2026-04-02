use std::collections::HashMap;

use goblin::mach::MachO;

use crate::shims;

#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    #[error("parse error: {0}")]
    Parse(String),
    #[error("unresolved symbol: {symbol} (from {dylib})")]
    Unresolved { symbol: String, dylib: String },
}

pub struct Linker {
    registry: HashMap<String, HashMap<String, u64>>,
}

impl Linker {
    pub fn new() -> Self {
        Self {
            registry: shims::default_registry(),
        }
    }

    /// Resolve all imports in the binary and patch the mapped memory.
    /// Segments must already be mmap'd at their vmaddrs.
    pub fn bind(&self, data: &[u8]) -> Result<usize, LinkError> {
        let macho = MachO::parse(data, 0)
            .map_err(|e| LinkError::Parse(e.to_string()))?;

        let imports = macho.imports()
            .map_err(|e| LinkError::Parse(e.to_string()))?;

        if imports.is_empty() {
            return Ok(0);
        }

        let mut bound = 0;
        for import in &imports {
            let dylib = import.dylib;
            let symbol = import.name;

            let addr = self.resolve(dylib, symbol, import.is_weak)?;

            // import.address is the VM address of the GOT/la_symbol_ptr slot
            let target_ptr = import.address as *mut u64;
            unsafe { target_ptr.write(addr) };

            log::debug!("bound {dylib}::{symbol} → {addr:#x} at {:#x}", import.address);
            bound += 1;
        }

        log::info!("bound {bound} imports");
        Ok(bound)
    }

    fn resolve(&self, dylib: &str, symbol: &str, is_weak: bool) -> Result<u64, LinkError> {
        if let Some(symbols) = self.registry.get(dylib) {
            if let Some(&addr) = symbols.get(symbol) {
                return Ok(addr);
            }
        }

        // Try short name match (strip path prefix)
        let short_name = dylib.rsplit('/').next().unwrap_or(dylib);
        for (reg_name, symbols) in &self.registry {
            if reg_name.ends_with(short_name) {
                if let Some(&addr) = symbols.get(symbol) {
                    return Ok(addr);
                }
            }
        }

        if is_weak {
            log::debug!("weak symbol {symbol} not found, binding to 0");
            return Ok(0);
        }

        Err(LinkError::Unresolved {
            symbol: symbol.into(),
            dylib: dylib.into(),
        })
    }
}

impl Default for Linker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_libsystem_write() {
        let linker = Linker::new();
        let addr = linker.resolve("/usr/lib/libSystem.B.dylib", "_write", false).unwrap();
        assert_ne!(addr, 0);
    }

    #[test]
    fn test_resolve_short_name() {
        let linker = Linker::new();
        let addr = linker.resolve("libSystem.B.dylib", "_exit", false).unwrap();
        assert_ne!(addr, 0);
    }

    #[test]
    fn test_weak_missing_returns_zero() {
        let linker = Linker::new();
        let addr = linker.resolve("/usr/lib/libSystem.B.dylib", "_nonexistent", true).unwrap();
        assert_eq!(addr, 0);
    }

    #[test]
    fn test_strong_missing_errors() {
        let linker = Linker::new();
        let result = linker.resolve("/usr/lib/libFoo.dylib", "_bar", false);
        assert!(result.is_err());
    }
}
