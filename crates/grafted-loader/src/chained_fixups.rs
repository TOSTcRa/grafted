//! LC_DYLD_CHAINED_FIXUPS support.

use crate::error::LoaderError;
use crate::macho::MachOBinary;

#[repr(C)]
struct dyld_chained_fixups_header {
    fixups_version: u32,
    starts_offset: u32,
    imports_offset: u32,
    symbols_offset: u32,
    imports_count: u32,
    imports_format: u32,
    symbols_format: u32,
}

#[repr(C)]
struct dyld_chained_starts_in_image {
    seg_count: u32,
}

#[repr(C)]
struct dyld_chained_starts_in_segment {
    size: u32,
    page_size: u16,
    pointer_format: u16,
    segment_offset: u64,
    max_valid_pointer: u32,
    page_count: u16,
}

pub fn apply_fixups<F>(binary: &MachOBinary, mut resolve_symbol: F) -> Result<(), LoaderError>
where
    F: FnMut(&str, &str) -> u64,
{
    let (fixups_offset, fixups_size) = match binary.chained_fixups {
        Some(x) => x,
        None => return Ok(()),
    };

    let fixups_data = &binary.data[fixups_offset as usize .. (fixups_offset + fixups_size) as usize];
    let header = unsafe { &*(fixups_data.as_ptr() as *const dyld_chained_fixups_header) };

    log::info!("applying chained fixups: version={}, imports_count={}, format={}", 
        header.fixups_version, header.imports_count, header.imports_format);

    let mut resolved_imports = Vec::with_capacity(header.imports_count as usize);
    let imports_ptr = fixups_data.as_ptr().wrapping_add(header.imports_offset as usize);
    
    let preferred_load_address = binary.segments.iter()
        .find(|s| s.name == "__TEXT")
        .map(|s| s.vmaddr)
        .unwrap_or(0);

    for i in 0..header.imports_count {
        let import_raw = unsafe { *(imports_ptr.wrapping_add(i as usize * 4) as *const u32) };
        // dyld_chained_import: lib_ordinal:8, weak_import:1, name_offset:23
        let lib_ordinal = (import_raw & 0xFF) as usize;
        let name_offset = header.symbols_offset + (import_raw >> 9);
        
        let name_ptr = fixups_data.as_ptr().wrapping_add(name_offset as usize);
        let name = unsafe { std::ffi::CStr::from_ptr(name_ptr as *const i8) }.to_str().unwrap();
        
        let dylib_name = if lib_ordinal > 0 && lib_ordinal < 0xFD && lib_ordinal <= binary.dylib_deps.len() {
            &binary.dylib_deps[lib_ordinal - 1].name
        } else {
            "self"
        };

        let addr = resolve_symbol(dylib_name, name);
        resolved_imports.push(addr);
    }

    let starts_ptr = fixups_data.as_ptr().wrapping_add(header.starts_offset as usize);
    let starts_in_image = unsafe { &*(starts_ptr as *const dyld_chained_starts_in_image) };

    for i in 0..starts_in_image.seg_count {
        let seg_info_offset = unsafe { *(starts_ptr.wrapping_add(4 + i as usize * 4) as *const u32) };
        if seg_info_offset == 0 { continue; }

        let seg_starts_ptr = starts_ptr.wrapping_add(seg_info_offset as usize);
        let seg_starts = unsafe { &*(seg_starts_ptr as *const dyld_chained_starts_in_segment) };

        if i as usize >= binary.segments.len() { continue; }
        let mapped_seg = &binary.segments[i as usize];
        if mapped_seg.name == "__PAGEZERO" { continue; }

        let page_starts_ptr = seg_starts_ptr.wrapping_add(22); // skip header to page_start array

        for j in 0..seg_starts.page_count {
            let page_start_offset = unsafe { *(page_starts_ptr.wrapping_add(j as usize * 2) as *const u16) };
            if page_start_offset == 0xFFFF { continue; }

            // Address of the first fixup on this page
            let mut chain_ptr = (mapped_seg.vmaddr + (j as u64 * seg_starts.page_size as u64) + page_start_offset as u64) as *mut u64;
            
            loop {
                let encoded = unsafe { *chain_ptr };
                let is_bind = (encoded >> 63) != 0;
                let next = (encoded >> 51) & 0xFFF; // 12 bits

                let addr = if is_bind {
                    // DYLD_CHAINED_PTR_64_BIND: ordinal:24, addend:8, reserved:19, next:12, bind:1
                    let ordinal = encoded & 0xFFFFFF;
                    if ordinal < resolved_imports.len() as u64 {
                        resolved_imports[ordinal as usize]
                    } else {
                        0
                    }
                } else {
                    // DYLD_CHAINED_PTR_64_REBASE: target:36, high8:8, reserved:7, next:12, bind:1
                    let target = encoded & 0xFFFFFFFFF;
                    let high8 = (encoded >> 36) & 0xFF;
                    let unpacked_target = (high8 << 56) | target;
                    preferred_load_address + unpacked_target
                };

                log::trace!("fixup at {:p}: bind={} raw={:#x} → {:#x}", chain_ptr, is_bind, encoded, addr);
                let page_base = (chain_ptr as usize & !0xFFF) as *mut libc::c_void;
                unsafe {
                    libc::mprotect(page_base, 4096, libc::PROT_READ | libc::PROT_WRITE);
                    *chain_ptr = addr;
                }

                if next == 0 { break; }
                chain_ptr = (chain_ptr as *mut u8).wrapping_add(next as usize * 4) as *mut u64;
            }
        }
    }

    Ok(())
}
