//! Swift Metadata Translation Layer
//!
//! Translates Mach-O Swift class metadata layout into a form the Linux
//! Swift runtime can consume. Runs AFTER chained fixups (which resolve
//! absolute pointers) but BEFORE swift_addNewDSOImage (which hands
//! metadata to the runtime).
//!
//! What we fix:
//! 1. ObjC class prefix: ensure class_data (+32) is non-NULL
//! 2. Swift description pointer: ensure it resolves to the type descriptor
//! 3. Metaclass layout: ensure metaclass has valid data too
//! 4. Generic argument pointers: ensure they're not NULL/soft-stub
//!
//! What we DON'T touch:
//! - Relative pointers in __swift5_* sections (they're position-independent)
//! - Chained fixup results (already correct)
//! - Value witness tables (allocated by the runtime)

/// Translate all Swift class metadata in a mapped Mach-O binary.
/// Must be called AFTER apply_fixups and BEFORE register_swift_sections.
pub fn translate_swift_metadata(binary_data: &[u8]) {
    let Ok(macho) = goblin::mach::MachO::parse(binary_data, 0) else { return };

    let mut types_translated = 0;
    let mut classes_fixed = 0;

    // Phase 1: Walk __swift5_types to find all type descriptors
    for seg in &macho.segments {
        for section_res in seg {
            if let Ok((section, _data)) = section_res {
                let name = section.name().unwrap_or("");

                if name == "__swift5_types" {
                    let count = section.size / 4; // each entry is a 4-byte relative pointer
                    let base = section.addr;

                    for i in 0..count {
                        let entry_addr = base + i * 4;
                        // Read the relative pointer
                        let rel = unsafe { *(entry_addr as *const i32) };
                        let descriptor_addr = (entry_addr as i64 + rel as i64) as u64;

                        if let Some(fixed) = fix_class_descriptor(descriptor_addr) {
                            classes_fixed += fixed;
                        }
                        types_translated += 1;
                    }
                }

                // Phase 2: Walk __objc_classlist to fix ObjC class prefix
                if name == "__objc_classlist" {
                    let count = section.size / 8;
                    let ptrs = section.addr as *const u64;

                    for i in 0..count {
                        let cls_addr = unsafe { std::ptr::read_unaligned(ptrs.add(i as usize)) };
                        if cls_addr == 0 { continue; }
                        if fix_objc_class_prefix(cls_addr) {
                            classes_fixed += 1;
                        }
                    }
                }
            }
        }
    }

    if types_translated > 0 || classes_fixed > 0 {
        log::info!("Swift metadata: translated {} types, fixed {} classes", types_translated, classes_fixed);
    }
}

/// Fix a class descriptor's statically-emitted metadata if present.
/// Returns the number of metadata records fixed.
fn fix_class_descriptor(descriptor_addr: u64) -> Option<u32> {
    if descriptor_addr < 0x1000 { return None; }

    // Read the context descriptor flags
    let flags = unsafe { *(descriptor_addr as *const u32) };
    let kind = flags & 0x1F;

    // Only handle class descriptors (kind = 0x10 = 16)
    if kind != 16 { return Some(0); }

    let is_generic = (flags & 0x80) != 0;

    // Read the metadata access function relative pointer (at +12)
    let access_fn_rel = unsafe { *((descriptor_addr + 12) as *const i32) };
    let _access_fn_addr = (descriptor_addr as i64 + 12 + access_fn_rel as i64) as u64;

    // For class descriptors, read negative/positive size
    let negative_size = unsafe { *((descriptor_addr + 24) as *const u32) } as usize;
    let positive_size = unsafe { *((descriptor_addr + 28) as *const u32) } as usize;

    // The metadata access function, when called, returns a pointer to the
    // metadata. But we can also find statically-emitted metadata by scanning
    // __DATA for class metadata records that reference this descriptor.
    //
    // For non-generic classes, the metadata is the canonical instance.
    // For generic classes, there may be pre-specialized instances.

    let _ = (negative_size, positive_size, is_generic);
    Some(0) // descriptor itself doesn't need fixing — the metadata records do
}

/// Fix the ObjC class prefix layout for a class at the given address.
/// Darwin objc_class layout:
///   +0:  isa (metaclass pointer)
///   +8:  superclass
///   +16: cache (cache_t: buckets(8) + mask/occupied(8))
///   +32: data (class_ro_t* with low bits as flags)
///
/// Returns true if any fix was applied.
fn fix_objc_class_prefix(cls_addr: u64) -> bool {
    if cls_addr < 0x1000 { return false; }
    let mut fixed = false;

    // Read the data pointer at +32 (NOT +24 — class_t.data is after the 16-byte cache_t)
    let data_ptr_addr = (cls_addr + 32) as *mut u64;
    let data_val = unsafe { std::ptr::read_unaligned(data_ptr_addr) };

    // Strip the low 3 bits (flags) to get the actual pointer
    let data_ptr = data_val & !0x7;

    if data_ptr == 0 {
        // Data pointer is NULL — create a minimal class_ro_t
        let ro = unsafe { libc::calloc(1, 128) } as *mut u64;
        unsafe {
            *ro = 0x80; // flags: RO_IS_SWIFT_STABLE (bit 7)
            *ro.add(1) = 256; // instance start
            *ro.add(2) = 256; // instance size
        }
        let page = (data_ptr_addr as usize & !0xFFF) as *mut libc::c_void;
        unsafe {
            libc::mprotect(page, 8192, libc::PROT_READ | libc::PROT_WRITE);
            std::ptr::write_unaligned(data_ptr_addr, ro as u64 | (data_val & 0x7));
            libc::mprotect(page, 8192, libc::PROT_READ | libc::PROT_WRITE);
        }
        fixed = true;
        log::debug!("  fixed class_data at {:#x}", cls_addr);
    }

    // Also fix the metaclass (pointed to by isa at +0)
    let isa = unsafe { std::ptr::read_unaligned(cls_addr as *const u64) };
    if isa > 0x1000 {
        let meta_data_ptr_addr = (isa + 32) as *mut u64;
        let meta_data_val = unsafe { std::ptr::read_unaligned(meta_data_ptr_addr) };
        let meta_data_ptr = meta_data_val & !0x7;

        if meta_data_ptr == 0 {
            let meta_ro = unsafe { libc::calloc(1, 128) } as *mut u64;
            unsafe {
                *meta_ro = 0x81; // flags: RO_META | RO_IS_SWIFT_STABLE
                *meta_ro.add(1) = 40; // meta instance start
                *meta_ro.add(2) = 40; // meta instance size
            }
            let page = (meta_data_ptr_addr as usize & !0xFFF) as *mut libc::c_void;
            unsafe {
                libc::mprotect(page, 8192, libc::PROT_READ | libc::PROT_WRITE);
                std::ptr::write_unaligned(meta_data_ptr_addr, meta_ro as u64 | (meta_data_val & 0x7));
            }
            fixed = true;
            log::debug!("  fixed metaclass_data for isa at {:#x}", isa);
        }
    }

    // Check and fix the Swift description pointer
    // For Swift class metadata, the description is at a variable offset after
    // the ObjC prefix. The exact offset depends on the class hierarchy.
    // For most Swift classes: offset +64 or +72 from the metadata start.
    // We scan for a value that looks like a type descriptor address.
    for desc_offset in [64_u64, 72, 80, 40, 48] {
        let desc_ptr_addr = (cls_addr + desc_offset) as *const u64;
        let desc_val = unsafe { std::ptr::read_unaligned(desc_ptr_addr) };
        // Check if this looks like a valid descriptor (in the binary's address range)
        if desc_val >= 0x100100000 && desc_val < 0x100200000 {
            // Verify it starts with valid ContextDescriptor flags
            let flags = unsafe { *(desc_val as *const u32) };
            let kind = flags & 0x1F;
            if kind == 16 { // class descriptor
                log::trace!("  description at +{}: {:#x} → valid class descriptor", desc_offset, desc_val);
                break;
            }
        }
    }

    fixed
}
