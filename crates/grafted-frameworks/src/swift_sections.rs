//! Register a Mach-O binary's __swift5_* sections with the Linux Swift runtime.
//!
//! The Swift runtime uses `swift_addNewDSOImage` to learn about type metadata,
//! protocol conformances, and type descriptors in loaded binaries. Since our
//! Mach-O binary is mmap'd (not loaded via dlopen), the runtime can't find it
//! automatically. We extract the section addresses and register them manually.

use std::ffi::CString;

/// A range of memory (start + length).
#[repr(C)]
struct SectionRange {
    start: *const u8,
    length: usize,
}

/// The struct passed to swift_addNewDSOImage.
/// Must EXACTLY match MetadataSections in MetadataSections.h.
#[repr(C)]
struct MetadataSections {
    version: usize,
    base_address: *const u8,           // __dso_handle / Mach header
    unused0: *const u8,
    unused1: *const u8,
    swift5_protocols: SectionRange,    // __swift5_protos
    swift5_protocol_conformances: SectionRange, // __swift5_proto
    swift5_type_metadata: SectionRange, // __swift5_types
    swift5_typeref: SectionRange,      // __swift5_typeref
    swift5_reflstr: SectionRange,      // __swift5_reflstr
    swift5_fieldmd: SectionRange,      // __swift5_fieldmd
    swift5_assocty: SectionRange,      // __swift5_assocty
    swift5_replace: SectionRange,
    swift5_replac2: SectionRange,
    swift5_builtin: SectionRange,      // __swift5_builtin
    swift5_capture: SectionRange,      // __swift5_capture
    swift5_mpenum: SectionRange,       // __swift5_mpenum
    swift5_accessible: SectionRange,   // __swift_as_entry
    swift5_runtime_attributes: SectionRange,
}

impl Default for SectionRange {
    fn default() -> Self { Self { start: std::ptr::null(), length: 0 } }
}

/// Register a Mach-O binary's Swift sections with the runtime.
/// Call this after mapping the binary and before entering the entry point.
pub fn register_swift_sections(binary_data: &[u8]) {
    // Find swift_addNewDSOImage in the loaded runtime
    let func_name = CString::new("swift_addNewDSOImage").unwrap();
    let func_ptr = unsafe { libc::dlsym(libc::RTLD_DEFAULT, func_name.as_ptr()) };
    if func_ptr.is_null() {
        log::debug!("swift_addNewDSOImage not found — Swift runtime not loaded");
        return;
    }
    let swift_add: unsafe extern "C" fn(*const MetadataSections) =
        unsafe { std::mem::transmute(func_ptr) };

    // Parse the Mach-O to find __swift5_* sections
    let Ok(macho) = goblin::mach::MachO::parse(binary_data, 0) else { return };

    // Find __TEXT vmaddr (= Mach header = __dso_handle equivalent)
    let base_addr = macho.segments.iter()
        .find(|s| s.name().unwrap_or("") == "__TEXT")
        .map(|s| s.vmaddr as *const u8)
        .unwrap_or(std::ptr::null());

    let mut sections = MetadataSections {
        version: 0,
        base_address: base_addr,
        unused0: std::ptr::null(),
        unused1: std::ptr::null(),
        swift5_protocols: SectionRange::default(),
        swift5_protocol_conformances: SectionRange::default(),
        swift5_type_metadata: SectionRange::default(),
        swift5_typeref: SectionRange::default(),
        swift5_reflstr: SectionRange::default(),
        swift5_fieldmd: SectionRange::default(),
        swift5_assocty: SectionRange::default(),
        swift5_replace: SectionRange::default(),
        swift5_replac2: SectionRange::default(),
        swift5_builtin: SectionRange::default(),
        swift5_capture: SectionRange::default(),
        swift5_mpenum: SectionRange::default(),
        swift5_accessible: SectionRange::default(),
        swift5_runtime_attributes: SectionRange::default(),
    };

    let mut found = 0;
    for seg in &macho.segments {
        for section_res in seg {
            if let Ok((section, _data)) = section_res {
                let name = section.name().unwrap_or("");
                let addr = section.addr as *const u8;
                let size = section.size as usize;

                match name {
                    "__swift5_protos" => { sections.swift5_protocols = SectionRange { start: addr, length: size }; found += 1; }
                    "__swift5_proto" => { sections.swift5_protocol_conformances = SectionRange { start: addr, length: size }; found += 1; }
                    "__swift5_types" => { sections.swift5_type_metadata = SectionRange { start: addr, length: size }; found += 1; }
                    "__swift5_typeref" => { sections.swift5_typeref = SectionRange { start: addr, length: size }; found += 1; }
                    "__swift5_reflstr" => { sections.swift5_reflstr = SectionRange { start: addr, length: size }; found += 1; }
                    "__swift5_fieldmd" => { sections.swift5_fieldmd = SectionRange { start: addr, length: size }; found += 1; }
                    "__swift5_assocty" => { sections.swift5_assocty = SectionRange { start: addr, length: size }; found += 1; }
                    "__swift5_builtin" => { sections.swift5_builtin = SectionRange { start: addr, length: size }; found += 1; }
                    "__swift5_capture" => { sections.swift5_capture = SectionRange { start: addr, length: size }; found += 1; }
                    "__swift5_mpenum" => { sections.swift5_mpenum = SectionRange { start: addr, length: size }; found += 1; }
                    "__swift_as_entry" => { sections.swift5_accessible = SectionRange { start: addr, length: size }; found += 1; }
                    _ => {}
                }
            }
        }
    }

    if found == 0 {
        log::debug!("No Swift metadata sections found in binary");
        return;
    }

    log::info!("Registering {} Swift metadata sections with runtime", found);
    unsafe { swift_add(&sections) };
    log::info!("Swift metadata sections registered successfully");
}
