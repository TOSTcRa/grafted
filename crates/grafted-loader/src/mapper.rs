//! Segment memory mapper.

use std::num::NonZeroUsize;
use std::ptr;

use nix::sys::mman::{mmap_anonymous, mprotect, MapFlags, ProtFlags};

use crate::error::LoaderError;
use crate::macho::{MachOBinary, Segment};

fn macho_prot_to_linux(prot: u32) -> ProtFlags {
    let mut flags = ProtFlags::empty();
    if prot & 1 != 0 {
        flags |= ProtFlags::PROT_READ;
    }
    if prot & 2 != 0 {
        flags |= ProtFlags::PROT_WRITE;
    }
    if prot & 4 != 0 {
        flags |= ProtFlags::PROT_EXEC;
    }
    flags
}

/// Map a segment. `slide` is added to vmaddr for position-independent dylibs.
fn map_segment(seg: &Segment, file_data: &[u8], slide: u64) -> Result<(), LoaderError> {
    if seg.vmsize == 0 {
        return Ok(());
    }

    let vmaddr = (seg.vmaddr + slide) as usize;
    let vmsize = seg.vmsize as usize;

    let length = NonZeroUsize::new(vmsize)
        .ok_or_else(|| LoaderError::Mmap("vmsize is zero".into()))?;

    let ptr = if vmaddr == 0 {
        // Kernel picks the address (shouldn't happen after slide, but safety fallback)
        unsafe {
            mmap_anonymous(
                None,
                length,
                ProtFlags::PROT_READ | ProtFlags::PROT_WRITE,
                MapFlags::MAP_PRIVATE,
            )
        }
    } else {
        let addr_hint = NonZeroUsize::new(vmaddr).unwrap();
        unsafe {
            mmap_anonymous(
                Some(addr_hint),
                length,
                ProtFlags::PROT_READ | ProtFlags::PROT_WRITE,
                MapFlags::MAP_PRIVATE | MapFlags::MAP_FIXED,
            )
        }
    }
    .map_err(|e| LoaderError::Mmap(format!("mmap at {vmaddr:#x}: {e}")))?;

    let fileoff = seg.fileoff as usize;
    let filesize = seg.filesize as usize;
    if filesize > 0 && fileoff + filesize <= file_data.len() {
        unsafe {
            ptr::copy_nonoverlapping(
                file_data[fileoff..].as_ptr(),
                ptr.as_ptr() as *mut u8,
                filesize,
            );
        }
    }

    let prot = macho_prot_to_linux(seg.initprot);
    unsafe {
        mprotect(ptr, vmsize, prot)
    }
    .map_err(|e| LoaderError::Mmap(format!("mprotect {}: {e}", seg.name)))?;

    log::debug!(
        "mapped segment {} at {:#x} ({:#x} bytes, prot={:?})",
        seg.name,
        vmaddr,
        vmsize,
        prot
    );

    Ok(())
}

/// Returns the resolved entry point address.
pub fn map_binary(binary: &MachOBinary) -> Result<u64, LoaderError> {
    let text_seg = binary.segments.iter().find(|s| s.name == "__TEXT");

    // Dylibs have __TEXT at vmaddr=0 — need a slide to a free address range.
    // Executables have __TEXT at a fixed address (e.g., 0x100000000).
    let slide = if text_seg.map(|s| s.vmaddr).unwrap_or(0) == 0 {
        // Allocate a region for the dylib, then unmap it (just to get an address)
        let total_size: u64 = binary.segments.iter()
            .filter(|s| s.name != "__PAGEZERO")
            .map(|s| s.vmaddr + s.vmsize)
            .max()
            .unwrap_or(0);
        if total_size == 0 {
            return Err(LoaderError::Mmap("dylib has no segments".into()));
        }
        let size = NonZeroUsize::new(total_size as usize).unwrap();
        let region = unsafe {
            mmap_anonymous(None, size, ProtFlags::PROT_NONE, MapFlags::MAP_PRIVATE)
        }.map_err(|e| LoaderError::Mmap(format!("dylib slide alloc: {e}")))?;
        let base = region.as_ptr() as u64;
        // Unmap — the individual segments will be mapped with MAP_FIXED at base + vmaddr
        unsafe { nix::sys::mman::munmap(region, total_size as usize).ok() };
        log::debug!("dylib slide: {base:#x} (total_size={total_size:#x})");
        base
    } else {
        0 // executable — no slide needed
    };

    for seg in &binary.segments {
        if seg.name == "__PAGEZERO" {
            continue;
        }
        map_segment(seg, &binary.data, slide)?;
    }

    // Update segment vmaddrs to reflect the slide (so symbol lookup uses slid addresses)
    // Note: we can't mutate binary.segments directly, but the mapped memory is correct.
    // The linker uses vmaddr from the binary struct, so we store the slide.

    let entry = if binary.entry_is_offset {
        let ts = text_seg.ok_or(LoaderError::NoTextSegment)?;
        let base_addr = (ts.vmaddr + slide) - ts.fileoff;
        base_addr + binary.entry_point
    } else {
        binary.entry_point + slide
    };

    log::info!("binary mapped, entry point at {entry:#x} (slide={slide:#x})");
    Ok(entry)
}
