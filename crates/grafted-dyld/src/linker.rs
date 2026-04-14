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

/// No-op value witness functions for the universal stub VWT.
unsafe extern "C" fn vwt_noop(_: *mut u8, _: *mut u8, _: *const u8) -> *mut u8 {
    std::ptr::null_mut()
}
unsafe extern "C" fn vwt_destroy(_: *mut u8, _: *const u8) {}
unsafe extern "C" fn vwt_initcopy(dest: *mut u8, src: *mut u8, _meta: *const u8) -> *mut u8 {
    unsafe { std::ptr::copy_nonoverlapping(src, dest, 8) };
    dest
}
unsafe extern "C" fn vwt_enum_tag(_: *const u8, _: u32, _: *const u8) -> u32 { 0 }
unsafe extern "C" fn vwt_store_enum_tag(_: *mut u8, _: u32, _: u32, _: *const u8) {}

/// Soft stub: returns a valid Swift metadata pointer with a proper VWT.
/// The VWT has valid function pointers and size/stride/flags so that
/// code reading metadata[-1]→VWT[0x40] (size) doesn't crash.
unsafe extern "C" fn shim_unresolved_soft() -> *mut u8 {
    static STUB_META: std::sync::atomic::AtomicPtr<u8> = std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());
    let mut ptr = STUB_META.load(std::sync::atomic::Ordering::Acquire);
    if ptr.is_null() {
        // Allocate: [VWT (88 bytes)] [metadata_prefix (8 bytes)] [metadata (64+ bytes)]
        let page = unsafe { libc::calloc(1, 8192) } as *mut u8;

        // Build the VWT at the start of the page
        let vwt = page as *mut u64;
        unsafe {
            *vwt.add(0) = vwt_initcopy as u64;  // initializeBufferWithCopyOfBuffer
            *vwt.add(1) = vwt_destroy as u64;    // destroy
            *vwt.add(2) = vwt_initcopy as u64;   // initializeWithCopy
            *vwt.add(3) = vwt_initcopy as u64;   // assignWithCopy
            *vwt.add(4) = vwt_initcopy as u64;   // initializeWithTake
            *vwt.add(5) = vwt_initcopy as u64;   // assignWithTake
            *vwt.add(6) = vwt_enum_tag as u64;   // getEnumTagSinglePayload
            *vwt.add(7) = vwt_store_enum_tag as u64; // storeEnumTagSinglePayload
            *vwt.add(8) = 8;                     // size
            *vwt.add(9) = 8;                     // stride
            // flags: alignment mask = 7 (8-byte aligned), BitwiseTakable | Inline
            *(vwt.add(10) as *mut u32) = 0x20007; // flags
            *((vwt.add(10) as *mut u32).add(1)) = 0; // extraInhabitantCount
        }

        // metadata_prefix: VWT pointer at offset -8 from the metadata pointer
        // metadata starts at page + 96 (VWT=88 bytes + 8 byte prefix)
        let meta_ptr = unsafe { page.add(96) };
        unsafe {
            // metadata[-1] = VWT pointer
            *((meta_ptr as *mut u64).sub(1)) = vwt as u64;
            // metadata[0] = kind (1 = struct, simple kind — NOT 0x200 which triggers
            // descriptor-based checks in swift_checkMetadataState)
            *(meta_ptr as *mut u64) = 1;
        }

        ptr = meta_ptr;
        STUB_META.store(ptr, std::sync::atomic::Ordering::Release);
    }
    ptr
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

            let mut lib_binary = match MachOBinary::from_path(&linux_path) {
                Ok(b) => b,
                Err(e) => {
                    log::warn!("failed to load {}: {e}", linux_path.display());
                    loaded.insert(dylib_path);
                    continue;
                }
            };
            
            let (_entry, slide) = grafted_loader::mapper::map_binary(&lib_binary)?;
            lib_binary.slide = slide;

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
        // ObjC class_data fixup is handled by swift_metadata_translate.rs.
        // Skip slid dylibs (vmaddr=0) — their pointers reference pre-slide addresses.
        for lib in self.loaded_libraries.values() {
            let text_vmaddr = lib.segments.iter().find(|s| s.name == "__TEXT").map(|s| s.vmaddr).unwrap_or(0);
            if text_vmaddr > 0 {
                if let Err(e) = self.register_objc_metadata(lib) {
                    log::debug!("ObjC metadata for {}: {}", lib.path, e);
                }
            }
        }
        // Register ObjC classes from the main binary (names only, skip method parsing).
        // This runs BEFORE translate_swift_metadata so class_data_bits at +32 is still valid.
        // Method dispatch for main binary classes is handled by grafted_lookup_method.
        if let Err(e) = self.register_objc_class_names(main_binary) {
            log::debug!("ObjC class names for main binary: {}", e);
        }

        // In Darwin, initializers are called in bottom-up order (dependencies first).
        // Skip slid libraries for now (init funcs also use pre-slide addresses).
        for lib in self.loaded_libraries.values() {
            let text_vmaddr = lib.segments.iter().find(|s| s.name == "__TEXT").map(|s| s.vmaddr).unwrap_or(0);
            if text_vmaddr > 0 {
                self.run_initializers(lib)?;
            }
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
                        let ptrs = (section.addr + binary.slide) as *const *mut grafted_objc::types::class_t;
                        log::info!("grafted-dyld: processing {} classes in {} (ptrs={:p}, slide={:#x})", count, binary.path, ptrs, binary.slide);
                        
                        for i in 0..count {
                            let cls_ptr = unsafe { std::ptr::read_unaligned(ptrs.add(i as usize)) };
                            if cls_ptr.is_null() { continue; }
                            
                            log::info!("  [{}/{}] processing class at {:p}", i, count, cls_ptr);
                            
                            unsafe {
                                let data_field_ptr = (cls_ptr as *const u8).add(
                                    std::mem::offset_of!(grafted_objc::types::class_t, data)
                                );
                                log::info!("    reading data_ptr from {:p}", data_field_ptr);
                                
                                let raw_data_ptr = std::ptr::read_unaligned(data_field_ptr as *const usize);
                                // Strip ObjC tag bits from class_data_bits (low 3 bits are flags)
                                let data_ptr = (raw_data_ptr & !7) as *mut grafted_objc::types::class_ro_t;
                                if data_ptr.is_null() {
                                    log::info!("    data_ptr is NULL, skipping");
                                    continue;
                                }

                                log::info!("    data_ptr={:p} (raw={:#x})", data_ptr, raw_data_ptr);

                                // class_ro_t layout: flags(4) + instanceStart(4) + instanceSize(4) +
                                // reserved(4) + ivarLayout(8) + name(8) → name is at +24
                                let class_name_ptr = std::ptr::read_unaligned((data_ptr as *const u8).add(24) as *const *const i8);
                                
                                let class_name = if !class_name_ptr.is_null() {
                                    log::info!("    class_name_ptr={:p}", class_name_ptr);
                                    std::ffi::CStr::from_ptr(class_name_ptr).to_string_lossy().into_owned()
                                } else {
                                    "Unknown".to_string()
                                };

                                log::info!("    detected Objective-C class: {}", class_name);

                                // class_ro_t: baseMethods at +32
                                let raw_method_list_ptr = std::ptr::read_unaligned((data_ptr as *const u8).add(32) as *const *mut grafted_objc::types::method_list_t);

                                // Modern ObjC tags the method_list_ptr in the lower bits
                                let method_list_ptr = ((raw_method_list_ptr as usize) & !3) as *mut grafted_objc::types::method_list_t;
                                
                                if !method_list_ptr.is_null() {
                                    let flags = std::ptr::read_unaligned(&mut (*method_list_ptr).entsize_and_flags);
                                    let method_count = std::ptr::read_unaligned(&mut (*method_list_ptr).count);
                                    
                                    let is_relative = (flags & 0x80000000) != 0;
                                    let entsize = flags & 0xFFFF;
                                    
                                    if method_count > 500 || entsize == 0 || entsize > 64 {
                                        log::warn!("  skipping class {} (methods={}, entsize={})", class_name, method_count, entsize);
                                        // Still register the class (without methods) so objc_getClass works
                                        grafted_objc::objc_registerClassPair(cls_ptr as *mut _ as grafted_objc::types::Class);
                                        continue;
                                    }

                                    let first_method_ptr = (method_list_ptr as *const u8).add(8);
                                    
                                    for m in 0..method_count {
                                        let method_base = first_method_ptr.add((m * entsize) as usize);
                                        
                                        let (sel, imp, types) = if is_relative {
                                            // Relative method list (method_t_rel)
                                            let name_offset = std::ptr::read_unaligned(method_base as *const i32);
                                            let types_offset = std::ptr::read_unaligned(method_base.add(4) as *const i32);
                                            let imp_offset = std::ptr::read_unaligned(method_base.add(8) as *const i32);
                                            
                                            let name_ptr = (method_base as *const i8).offset(name_offset as isize);
                                            let sel = grafted_objc::sel_registerName(name_ptr);
                                            
                                            let imp_ptr = (method_base.add(8) as *const i8).offset(imp_offset as isize);
                                            let types_ptr = (method_base.add(4) as *const i8).offset(types_offset as isize);
                                            
                                            (sel, Some(std::mem::transmute(imp_ptr)), types_ptr)
                                        } else {
                                            // Absolute method list
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

    /// Lightweight ObjC class registration: register class names only, skip method parsing.
    /// This is used for the main binary where full method list parsing can hang.
    fn register_objc_class_names(&self, binary: &MachOBinary) -> Result<(), LinkError> {
        let macho = MachO::parse(&binary.data, 0)
            .map_err(|e| LinkError::Parse(e.to_string()))?;

        for segment in &macho.segments {
            for section_res in segment {
                if let Ok((section, _data)) = section_res {
                    let sect_name = section.name().unwrap_or("");
                    if sect_name == "__objc_classlist" {
                        let count = section.size / 8;
                        let ptrs = (section.addr + binary.slide) as *const *mut grafted_objc::types::class_t;
                        log::info!("grafted-dyld: registering {} class names from main binary", count);

                        for i in 0..count {
                            let cls_ptr = unsafe { std::ptr::read_unaligned(ptrs.add(i as usize)) };
                            if cls_ptr.is_null() { continue; }

                            unsafe {
                                // Read class_data_bits at +32, strip tag bits
                                let raw_bits = std::ptr::read_unaligned((cls_ptr as *const u8).add(32) as *const usize);
                                let data_ptr = (raw_bits & !7) as *const u8;
                                if data_ptr.is_null() { continue; }

                                // class_ro_t name at +24
                                let name_ptr = std::ptr::read_unaligned(data_ptr.add(24) as *const *const i8);
                                if name_ptr.is_null() { continue; }

                                let class_name = std::ffi::CStr::from_ptr(name_ptr).to_string_lossy();

                                // Register the class name first
                                grafted_objc::objc_registerClassPair(cls_ptr as *mut _ as grafted_objc::types::Class);

                                // Now register methods from class_ro_t baseMethods at +32
                                let method_list_raw = std::ptr::read_unaligned(data_ptr.add(32) as *const usize);
                                let method_list_ptr = (method_list_raw & !3) as *const u8;
                                let mut method_count_registered = 0u32;

                                if !method_list_ptr.is_null() && (method_list_ptr as usize) > 0x10000 {
                                    let flags = std::ptr::read_unaligned(method_list_ptr as *const u32);
                                    let m_count = std::ptr::read_unaligned(method_list_ptr.add(4) as *const u32);
                                    let is_relative = (flags & 0x80000000) != 0;
                                    let entsize = flags & 0xFFFF;

                                    if m_count <= 200 && entsize > 0 && entsize <= 64 {
                                        let first_method = method_list_ptr.add(8);
                                        for m in 0..m_count {
                                            let method_base = first_method.add((m * entsize) as usize);

                                            if is_relative {
                                                let name_offset = std::ptr::read_unaligned(method_base as *const i32);
                                                let imp_offset = std::ptr::read_unaligned(method_base.add(8) as *const i32);

                                                let name_ptr = (method_base as *const i8).offset(name_offset as isize);
                                                let imp_addr = (method_base.add(8) as *const i8).offset(imp_offset as isize);

                                                let sel = grafted_objc::sel_registerName(name_ptr);
                                                grafted_objc::class_addMethod(
                                                    cls_ptr as *mut _ as grafted_objc::types::Class,
                                                    sel,
                                                    Some(std::mem::transmute(imp_addr)),
                                                    std::ptr::null(),
                                                );
                                                method_count_registered += 1;
                                            } else {
                                                let name_ptr = std::ptr::read_unaligned(method_base as *const *const i8);
                                                let imp_ptr = std::ptr::read_unaligned(method_base.add(16) as *const *const ());

                                                let sel = grafted_objc::sel_registerName(name_ptr);
                                                grafted_objc::class_addMethod(
                                                    cls_ptr as *mut _ as grafted_objc::types::Class,
                                                    sel,
                                                    Some(std::mem::transmute(imp_ptr)),
                                                    std::ptr::null(),
                                                );
                                                method_count_registered += 1;
                                            }
                                        }
                                    }
                                }

                                log::info!("  registered ObjC class: {} ({} methods)", class_name, method_count_registered);
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
                // Use soft stub for Swift mangled symbols (there are thousands).
                // Hard trap only for symbols that are likely to be called immediately.
                if symbol.starts_with("_$s") || symbol.starts_with("$s") {
                    log::trace!("soft stub: {symbol}");
                    shim_unresolved_soft as *const () as u64
                } else {
                    log::warn!("chained fixup: unresolved symbol {symbol} from {dylib}");
                    shim_unresolved_soft as *const () as u64
                }
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

        // Auto-generate stub ObjC classes for _OBJC_CLASS_$_ and _OBJC_METACLASS_$_ symbols.
        // Darwin binaries reference classes by these symbols; we create minimal stubs
        // registered with our ObjC runtime so objc_msgSend can dispatch to them.
        if let Some(class_name) = symbol.strip_prefix("_OBJC_CLASS_$_") {
            let cls = auto_create_objc_class(class_name);
            if !cls.is_null() {
                return Ok(cls as u64);
            }
        }
        if let Some(class_name) = symbol.strip_prefix("_OBJC_METACLASS_$_") {
            // Metaclass — for simplicity, return the same class pointer
            let cls = auto_create_objc_class(class_name);
            if !cls.is_null() {
                return Ok(cls as u64);
            }
        }

        // Try dlsym as last resort — Swift runtime libraries are loaded with RTLD_GLOBAL,
        // so Foundation/Observation/etc. symbols are in the global symbol table.
        if symbol.starts_with("_$s") || symbol.starts_with("$s") || symbol.starts_with("_swift_") {
            let clean = symbol.strip_prefix('_').unwrap_or(symbol);
            let c_name = std::ffi::CString::new(clean).unwrap();
            let addr = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c_name.as_ptr()) };
            if !addr.is_null() {
                log::debug!("dlsym fallback resolved: {symbol} → {:#x}", addr as u64);
                return Ok(addr as u64);
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

/// Auto-create a stub ObjC class and register it with the runtime.
/// Returns the class pointer, or null if already registered.
fn auto_create_objc_class(name: &str) -> grafted_objc::types::Class {
    use std::sync::Mutex;
    use std::collections::HashMap as HMap;

    // Class pointers are process-lifetime allocations — safe to share across threads.
    struct SendClass(HMap<String, grafted_objc::types::Class>);
    unsafe impl Send for SendClass {}
    static CLASS_CACHE: Mutex<Option<SendClass>> = Mutex::new(None);

    let mut cache = CLASS_CACHE.lock().unwrap();
    let map = &mut cache.get_or_insert_with(|| SendClass(HMap::new())).0;

    if let Some(&cls) = map.get(name) {
        return cls;
    }

    // Check if already registered in the ObjC runtime
    let c_name = std::ffi::CString::new(name).unwrap_or_default();
    let existing = grafted_objc::objc_getClass(c_name.as_ptr());
    if !existing.is_null() {
        log::debug!("auto_create_objc_class: reusing registered class {} → {:p}", name, existing);
        map.insert(name.to_string(), existing);
        return existing;
    }
    log::debug!("auto_create_objc_class: creating stub for {}", name);

    // Allocate a new stub class
    let cls = unsafe { libc::calloc(1, 256) } as grafted_objc::types::Class;
    let ro = unsafe { libc::calloc(1, 256) } as *mut grafted_objc::types::class_ro_t;
    let leaked_name = c_name.into_raw();
    unsafe {
        (*ro).name = leaked_name;
        (*ro).instance_size = 256; // generous default
        (*(cls as *mut grafted_objc::types::class_t)).data = ro;
    }
    grafted_objc::objc_registerClassPair(cls);

    log::debug!("auto-created ObjC stub class: {name}");
    map.insert(name.to_string(), cls);
    cls
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
