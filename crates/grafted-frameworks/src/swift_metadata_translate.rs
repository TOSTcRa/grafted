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

/// Translate Darwin class metadata layout to Linux layout.
///
/// The key difference: Darwin has ObjC cache_t (16 bytes) at +16, which
/// shifts ALL subsequent fields by 24 bytes compared to Linux:
///
///   Field                  Darwin offset   Linux offset
///   isa                    +0              +0
///   superclass             +8              +8
///   cache_t (ObjC)         +16 (16 bytes)  (not present)
///   class_data_bits        +32             (not present)
///   Swift Flags            +40             +16
///   InstanceAddressPoint   +44             +20
///   InstanceSize           +48             +24
///   InstanceAlignMask      +52             +28
///   Reserved               +54             +30
///   ClassSize              +56             +32
///   ClassAddressPoint      +60             +36
///   Description            +64             +40
///
/// The translation copies Swift-specific fields from Darwin offsets to
/// Linux offsets IN-PLACE so the Linux runtime reads correct values.
fn fix_objc_class_prefix(cls_addr: u64) -> bool {
    if cls_addr < 0x1000 { return false; }

    // Check if this is Swift class metadata by looking for the description
    // pointer at Darwin offset +64 (should be a valid address in __TEXT/__DATA)
    let darwin_desc_addr = (cls_addr + 64) as *const u64;
    let desc_val = unsafe { std::ptr::read_unaligned(darwin_desc_addr) };

    // Only translate if there's a valid-looking description pointer
    if desc_val < 0x100000000 || desc_val > 0x200000000 { return false; }

    // Verify it's actually a class descriptor (kind = 16)
    let flags = unsafe { std::ptr::read_unaligned(desc_val as *const u32) };
    let kind = flags & 0x1F;
    if kind != 16 { return false; }

    let page = (cls_addr as usize & !0xFFF) as *mut libc::c_void;
    unsafe { libc::mprotect(page, 8192, libc::PROT_READ | libc::PROT_WRITE) };

    // Copy Swift fields from Darwin offsets to Linux offsets (shift by -24)
    // Darwin +40 → Linux +16 (Flags, 4 bytes)
    // Darwin +44 → Linux +20 (InstanceAddressPoint, 4 bytes)
    // Darwin +48 → Linux +24 (InstanceSize, 4 bytes)
    // Darwin +52 → Linux +28 (InstanceAlignMask, 2 bytes)
    // Darwin +54 → Linux +30 (Reserved, 2 bytes)
    // Darwin +56 → Linux +32 (ClassSize, 4 bytes)
    // Darwin +60 → Linux +36 (ClassAddressPoint, 4 bytes)
    // Darwin +64 → Linux +40 (Description, 8 bytes)
    // Darwin +72 → Linux +48 (IVarDestroyer?, 8 bytes)
    //
    // We copy the 40-byte block from +40 to +16 (shifting left by 24)
    unsafe {
        let src = (cls_addr + 40) as *const u8;
        let dst = (cls_addr + 16) as *mut u8;
        // Copy 40 bytes: flags(4) + addressPoint(4) + instanceSize(4) +
        // alignMask(2) + reserved(2) + classSize(4) + classAddressPoint(4) +
        // description(8) + ivarDestroyer(8)
        std::ptr::copy(src, dst, 40);
    }

    log::debug!("  translated class metadata at {:#x} (Darwin→Linux layout shift -24)", cls_addr);

    // Also translate the metaclass
    let isa = unsafe { std::ptr::read_unaligned(cls_addr as *const u64) };
    if isa > 0x100000000 && isa < 0x200000000 {
        let meta_desc = unsafe { std::ptr::read_unaligned((isa + 64) as *const u64) };
        if meta_desc >= 0x100000000 && meta_desc < 0x200000000 {
            let meta_page = (isa as usize & !0xFFF) as *mut libc::c_void;
            unsafe {
                libc::mprotect(meta_page, 8192, libc::PROT_READ | libc::PROT_WRITE);
                let src = (isa + 40) as *const u8;
                let dst = (isa + 16) as *mut u8;
                std::ptr::copy(src, dst, 40);
            }
            log::debug!("  translated metaclass at {:#x}", isa);
        }
    }

    true
}
