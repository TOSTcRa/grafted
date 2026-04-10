use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use goblin::mach::MachO;
use grafted_loader::{MachOBinary, LoaderError};

use crate::{shims, resolver::DylibResolver};

unsafe extern "C" fn shim_unresolved_trap() -> ! {
    let msg = b"grafted: called unresolved symbol\n";
    unsafe { libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len()) };
    unsafe { libc::_exit(127) };
}

#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    #[error("parse error: {0}")]
    Parse(String),
    #[error("unresolved symbol: {symbol} (from {dylib})")]
    Unresolved { symbol: String, dylib: String },
    #[error("loader error: {0}")]
    Loader(#[from] LoaderError),
    #[error("resolver error: library {0} not found")]
    LibraryNotFound(String),
}

pub struct Linker {
    registry: HashMap<String, HashMap<String, u64>>,
    resolver: DylibResolver,
    loaded_libraries: HashMap<String, MachOBinary>,
}

impl Linker {
    pub fn new() -> Self {
        let mut registry = shims::default_registry();

        // Merge framework symbols (CoreFoundation, CoreGraphics, AppKit, Swift, etc.)
        for (path, symbols) in grafted_frameworks::registry::framework_registry() {
            registry.entry(path).or_default().extend(symbols);
        }

        Self {
            registry,
            resolver: DylibResolver::default(),
            loaded_libraries: HashMap::new(),
        }
    }

    pub fn add_shim_dir(&mut self, dir: PathBuf) {
        self.resolver.add_shim_dir(dir);
    }

    pub fn add_override(&mut self, darwin_path: String, linux_path: PathBuf) {
        self.resolver.add_override(darwin_path, linux_path);
    }

    /// Recursively load and map all dependencies of the binary.
    pub fn load_dependencies(&mut self, binary: &MachOBinary) -> Result<(), LinkError> {
        // Keep track of paths to process. Tuple is (dylib_to_load, parent_path, parent_rpaths)
        let mut to_load: Vec<(String, PathBuf, Vec<String>)> = binary
            .dylib_deps
            .iter()
            .map(|d| (d.name.clone(), PathBuf::from(&binary.path), binary.rpaths.clone()))
            .collect();

        let mut loaded: HashSet<String> = HashSet::new();
        let executable_path = PathBuf::from(&binary.path);

        while let Some((dylib_path, loader_path, rpaths)) = to_load.pop() {
            if loaded.contains(&dylib_path) || self.registry.contains_key(&dylib_path) {
                continue;
            }

            log::info!("resolving library: {}", dylib_path);
            let linux_path = match self.resolver.resolve(
                &dylib_path,
                Some(&executable_path),
                Some(&loader_path),
                &rpaths,
            ) {
                Some(p) => p,
                None => {
                    log::warn!("library not found, skipping: {dylib_path}");
                    loaded.insert(dylib_path);
                    continue;
                }
            };

            let lib_binary = match MachOBinary::from_path(&linux_path) {
                Ok(b) => b,
                Err(e) => {
                    log::warn!("failed to load {}: {e}", linux_path.display());
                    loaded.insert(dylib_path);
                    continue;
                }
            };
            
            grafted_loader::mapper::map_binary(&lib_binary)?;

            for dep in &lib_binary.dylib_deps {
                to_load.push((dep.name.clone(), linux_path.clone(), lib_binary.rpaths.clone()));
            }

            self.loaded_libraries.insert(dylib_path.clone(), lib_binary);
            loaded.insert(dylib_path);
        }

        Ok(())
    }

    /// Resolve all imports in the binary and patch the mapped memory.
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

            let addr = match self.resolve(dylib, symbol, import.is_weak) {
                Ok(a) => a,
                Err(_) => {
                    log::warn!("unresolved symbol {symbol}, binding to trap");
                    shim_unresolved_trap as *const () as u64
                }
            };

            let target_ptr = import.address as *mut u64;
            
            let page_addr = (import.address & !0xFFF) as *mut libc::c_void;
            unsafe {
                // In a real dyld we'd track original protections, but for now we 
                // just ensure it's writable then readable.
                libc::mprotect(page_addr, 4096, libc::PROT_READ | libc::PROT_WRITE);
                target_ptr.write(addr);
            };

            log::debug!("bound {dylib}::{symbol} → {addr:#x} at {:#x}", import.address);
            bound += 1;
        }

        log::info!("bound {bound} imports");
        Ok(bound)
    }

    /// Resolve __nl_symbol_ptr entries for binaries that use non-lazy symbol pointers
    /// (e.g., Go binaries). Uses the indirect symbol table to map each nl_symbol_ptr
    /// entry to the correct symbol. Go's linker sometimes doesn't set indirectsymoff/
    /// nindirectsyms in LC_DYSYMTAB but still places the data after the symbol table.
    pub fn bind_nl_symbol_ptrs(&self, binary: &MachOBinary) -> Result<usize, LinkError> {
        let macho = MachO::parse(&binary.data, 0)
            .map_err(|e| LinkError::Parse(e.to_string()))?;

        // Find __nl_symbol_ptr section
        let mut nl_ptr_addr: u64 = 0;
        let mut nl_ptr_count: u64 = 0;
        for seg in &macho.segments {
            for section_res in seg {
                if let Ok((section, _)) = section_res {
                    if section.flags & 0xFF == 0x6 { // S_NON_LAZY_SYMBOL_POINTERS
                        nl_ptr_addr = section.addr;
                        nl_ptr_count = section.size / 8;
                    }
                }
            }
        }
        if nl_ptr_count == 0 { return Ok(0); }

        // Get symtab/dysymtab info
        let mut symtab_off = 0u32;
        let mut nsyms = 0u32;
        let mut strtab_off = 0u32;
        let mut indirect_off = 0u32;
        let mut nindirect = 0u32;
        for lc in &macho.load_commands {
            match &lc.command {
                goblin::mach::load_command::CommandVariant::Symtab(s) => {
                    symtab_off = s.symoff; nsyms = s.nsyms; strtab_off = s.stroff;
                }
                goblin::mach::load_command::CommandVariant::Dysymtab(d) => {
                    indirect_off = d.indirectsymoff; nindirect = d.nindirectsyms;
                }
                _ => {}
            }
        }

        // Find the indirect table: prefer the header, fall back to after-symtab
        let indirect_data_off = if indirect_off > 0 && nindirect > 0 {
            indirect_off as usize
        } else {
            // Go's linker places it right after the symbol table
            (symtab_off as usize) + (nsyms as usize) * 16
        };

        // For __symbol_stub1, indirect entries start at index 0.
        // For __nl_symbol_ptr, indirect entries start at nl_reserved1.
        // The stub entries (0..nl_reserved1) and nl_ptr entries (nl_reserved1..)
        // use the SAME indirect table. stub[k] uses nl_ptr[k], and
        // indirect[k] gives the symbol for stub[k].
        // So nl_ptr[k] should contain the resolved address for symbol at indirect[k].

        let symbols: Vec<_> = macho.symbols().collect();
        let mut bound = 0;

        log::debug!("bind_nl_symbol_ptrs: {} entries, indirect at file offset {:#x}, stub_start_idx=0",
            nl_ptr_count, indirect_data_off);

        for k in 0..nl_ptr_count as usize {
            // Read indirect table entry: 4-byte symbol table index
            let entry_off = indirect_data_off + k * 4;
            if entry_off + 4 > binary.data.len() { break; }
            let sym_idx = u32::from_le_bytes([
                binary.data[entry_off],
                binary.data[entry_off + 1],
                binary.data[entry_off + 2],
                binary.data[entry_off + 3],
            ]) as usize;

            // Validate and look up symbol name
            if sym_idx >= symbols.len() { continue; }
            let (name, _nlist) = match &symbols[sym_idx] {
                Ok(s) => s,
                Err(_) => continue,
            };

            let dylib = "/usr/lib/libSystem.B.dylib";
            let addr = match self.resolve(dylib, name, false) {
                Ok(a) => a,
                Err(_) => {
                    log::warn!("nl_symbol_ptr: unresolved {name}, binding to trap");
                    shim_unresolved_trap as *const () as u64
                }
            };

            let target_addr = nl_ptr_addr + (k as u64) * 8;
            let page = (target_addr & !0xFFF) as *mut libc::c_void;
            unsafe {
                libc::mprotect(page, 4096, libc::PROT_READ | libc::PROT_WRITE);
                (target_addr as *mut u64).write(addr);
            }

            log::debug!("nl_sym[{k}] {name} → {addr:#x} at {target_addr:#x}");
            bound += 1;
        }

        log::info!("bound {bound} nl_symbol_ptr entries");
        Ok(bound)
    }

    /// Run initialization functions (__mod_init_func and LC_ROUTINES).
    pub fn run_initializers(&self, binary: &MachOBinary) -> Result<(), LinkError> {
        let macho = MachO::parse(&binary.data, 0)
            .map_err(|e| LinkError::Parse(e.to_string()))?;

        // 1. Call LC_ROUTINES_64 if present
        for lc in &macho.load_commands {
            if let goblin::mach::load_command::CommandVariant::Routines64(r) = &lc.command {
                log::info!("calling LC_ROUTINES_64 at {:#x}", r.init_address);
                let f: extern "C" fn() = unsafe { std::mem::transmute(r.init_address) };
                f();
            }
        }

        // 2. Call functions in __mod_init_func sections
        for segment in &macho.segments {
            for item in segment {
                if let Ok((section, _data)) = item {
                    let sect_name = section.name().unwrap_or("");
                    if sect_name == "__mod_init_func" {
                        let start = section.addr as *const extern "C" fn();
                        let count = section.size / 8;
                        log::info!("calling {} functions in __mod_init_func", count);
                        for i in 0..count {
                            let f = unsafe { *start.add(i as usize) };
                            log::debug!("calling init func at {:?}", f);
                            f();
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Run initializers for all loaded libraries, then the main binary.
    pub fn run_all_initializers(&self, main_binary: &MachOBinary) -> Result<(), LinkError> {
        // Before running initializers, register all ObjC classes
        for lib in self.loaded_libraries.values() {
            self.register_objc_metadata(lib)?;
        }
        self.register_objc_metadata(main_binary)?;

        // In Darwin, initializers are called in bottom-up order (dependencies first).
        for lib in self.loaded_libraries.values() {
            self.run_initializers(lib)?;
        }
        self.run_initializers(main_binary)?;
        Ok(())
    }

    fn register_objc_metadata(&self, binary: &MachOBinary) -> Result<(), LinkError> {
        let macho = MachO::parse(&binary.data, 0)
            .map_err(|e| LinkError::Parse(e.to_string()))?;

        for segment in &macho.segments {
            for section_res in segment {
                if let Ok((section, _data)) = section_res {
                    let sect_name = section.name().unwrap_or("");
                    if sect_name == "__objc_classlist" {
                        let count = section.size / 8;
                        let ptrs = section.addr as *const *mut grafted_objc::types::class_t;
                        log::info!("grafted-dyld: processing {} classes in {}", count, binary.path);
                        
                        for i in 0..count {
                            let cls_ptr = unsafe { *ptrs.add(i as usize) };
                            if cls_ptr.is_null() { continue; }
                            
                            unsafe {
                                let data_ptr = (*cls_ptr).data;
                                if data_ptr.is_null() { continue; }
                                
                                let class_name_ptr = (*data_ptr).name;
                                let class_name = if !class_name_ptr.is_null() {
                                    std::ffi::CStr::from_ptr(class_name_ptr).to_string_lossy().into_owned()
                                } else {
                                    "Unknown".to_string()
                                };

                                log::debug!("detected Objective-C class: {}", class_name);

                                let raw_method_list_ptr = (*data_ptr).base_methods;
                                
                                // Modern ObjC tags the method_list_ptr in the lower bits
                                let method_list_ptr = ((raw_method_list_ptr as usize) & !3) as *mut grafted_objc::types::method_list_t;
                                log::debug!("  class_ro_t: flags={:#x}, name={:p}, base_methods_raw={:p}, base_methods_aligned={:p}", (*data_ptr).flags, (*data_ptr).name, raw_method_list_ptr, method_list_ptr);
                                
                                if !method_list_ptr.is_null() {
                                    let flags = std::ptr::read_unaligned(&mut (*method_list_ptr).entsize_and_flags);
                                    let method_count = std::ptr::read_unaligned(&mut (*method_list_ptr).count);
                                    
                                    let is_relative = (flags & 0x80000000) != 0;
                                    let entsize = flags & 0xFFFF;
                                    
                                    log::debug!("  found {} methods in {} (relative: {}, entsize: {})", method_count, class_name, is_relative, entsize);
                                    
                                    let first_method_ptr = (method_list_ptr as *const u8).add(8);
                                    
                                    for m in 0..method_count {
                                        let method_base = first_method_ptr.add((m * entsize) as usize);
                                        
                                        let (sel, imp, types) = if is_relative {
                                            // Relative method list (method_t_rel)
                                            let name_offset = std::ptr::read_unaligned(method_base as *const i32);
                                            let types_offset = std::ptr::read_unaligned(method_base.add(4) as *const i32);
                                            let imp_offset = std::ptr::read_unaligned(method_base.add(8) as *const i32);
                                            
                                            let name_ptr = method_base.offset(name_offset as isize);
                                            let sel = grafted_objc::sel_registerName(name_ptr as *const i8);
                                            
                                            let imp_ptr = method_base.add(8).offset(imp_offset as isize);
                                            let types_ptr = method_base.add(4).offset(types_offset as isize) as *const i8;
                                            
                                            (sel, Some(std::mem::transmute(imp_ptr)), types_ptr)
                                        } else {
                                            // Absolute method list
                                            // DO NOT cast to method_t reference if it's unaligned. Read fields manually.
                                            let name_ptr = std::ptr::read_unaligned(method_base as *const *const i8);
                                            let types_ptr = std::ptr::read_unaligned(method_base.add(8) as *const *const i8);
                                            let imp_ptr = std::ptr::read_unaligned(method_base.add(16) as *const *const ());
                                            
                                            let sel = grafted_objc::sel_registerName(name_ptr);
                                            (sel, Some(std::mem::transmute(imp_ptr)), types_ptr)
                                        };
                                        
                                        grafted_objc::class_addMethod(
                                            cls_ptr as *mut _ as grafted_objc::types::Class,
                                            sel,
                                            imp,
                                            types
                                        );
                                        
                                        log::trace!("    registered method at {:?}", imp);
                                    }
                                }

                                grafted_objc::objc_registerClassPair(cls_ptr as *mut _ as grafted_objc::types::Class);
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Public interface for resolving a symbol from a specific dylib (or global search).
    pub fn resolve_external(&self, dylib: &str, symbol: &str) -> u64 {
        match self.resolve(dylib, symbol, false) {
            Ok(addr) => addr,
            Err(_) => {
                log::warn!("chained fixup: unresolved symbol {symbol} from {dylib}");
                shim_unresolved_trap as *const () as u64
            }
        }
    }

    fn resolve(&self, dylib: &str, symbol: &str, is_weak: bool) -> Result<u64, LinkError> {
        if let Some(symbols) = self.registry.get(dylib) {
            if let Some(&addr) = symbols.get(symbol) {
                return Ok(addr);
            }
        }

        let short_name = dylib.rsplit('/').next().unwrap_or(dylib);
        for (reg_name, symbols) in &self.registry {
            if reg_name.ends_with(short_name) {
                if let Some(&addr) = symbols.get(symbol) {
                    return Ok(addr);
                }
            }
        }

        if let Some(lib) = self.loaded_libraries.get(dylib) {
            if let Some(addr) = self.find_symbol_in_binary(lib, symbol) {
                return Ok(addr);
            }
        }

        for lib in self.loaded_libraries.values() {
            if let Some(addr) = self.find_symbol_in_binary(lib, symbol) {
                return Ok(addr);
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

    fn find_symbol_in_binary(&self, binary: &MachOBinary, symbol: &str) -> Option<u64> {
        let macho = MachO::parse(&binary.data, 0).ok()?;
        
        let text_seg = binary.segments.iter().find(|s| s.name == "__TEXT")?;
        let base_addr = text_seg.vmaddr;

        // Modern Mach-O: Check Export Trie first (much faster and more reliable for dylibs)
        if let Ok(exports) = macho.exports() {
            for export in exports {
                if export.name == symbol {
                    // export.offset is relative to the start of the Mach-O header
                    // in our case, the header is at the start of the __TEXT segment.
                    return Some(base_addr + export.offset);
                }
            }
        }

        // Fallback: Legacy Symbol Table
        for sym in macho.symbols() {
            if let Ok((name, nlist)) = sym {
                // Mach symbols often have a leading underscore (e.g., _write)
                if (name == symbol || name.strip_prefix('_') == Some(symbol)) && nlist.n_value != 0 {
                    return Some(nlist.n_value);
                }
            }
        }

        None
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
