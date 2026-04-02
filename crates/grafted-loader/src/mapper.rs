//! Segment memory mapper.
//!
//! Maps Mach-O segments into memory at their specified virtual addresses
//! using mmap(MAP_FIXED). Skips __PAGEZERO (guard page).

use std::num::NonZeroUsize;
use std::ptr;

use nix::sys::mman::{mmap_anonymous, mprotect, MapFlags, ProtFlags};

use crate::error::LoaderError;
use crate::macho::{MachOBinary, Segment};

/// Convert Mach-O protection flags to Linux mmap ProtFlags.
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

/// Map a single segment into memory at its vmaddr.
fn map_segment(seg: &Segment, file_data: &[u8]) -> Result<(), LoaderError> {
    if seg.vmsize == 0 {
        return Ok(());
    }

    let vmaddr = seg.vmaddr as usize;
    let vmsize = seg.vmsize as usize;

    let addr_hint = NonZeroUsize::new(vmaddr)
        .ok_or_else(|| LoaderError::Mmap("vmaddr is zero".into()))?;
    let length = NonZeroUsize::new(vmsize)
        .ok_or_else(|| LoaderError::Mmap("vmsize is zero".into()))?;

    let ptr = unsafe {
        mmap_anonymous(
            Some(addr_hint),
            length,
            ProtFlags::PROT_READ | ProtFlags::PROT_WRITE,
            MapFlags::MAP_PRIVATE | MapFlags::MAP_FIXED,
        )
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

/// Map all segments of a Mach-O binary into memory.
/// Skips __PAGEZERO (null guard page).
/// Returns the resolved entry point address.
pub fn map_binary(binary: &MachOBinary) -> Result<u64, LoaderError> {
    for seg in &binary.segments {
        if seg.name == "__PAGEZERO" {
            log::debug!("skipping __PAGEZERO segment");
            continue;
        }

        map_segment(seg, &binary.data)?;
    }

    let entry = if binary.entry_is_offset {
        // LC_MAIN: offset relative to __TEXT vmaddr
        let text_seg = binary
            .segments
            .iter()
            .find(|s| s.name == "__TEXT")
            .ok_or(LoaderError::NoTextSegment)?;
        text_seg.vmaddr + binary.entry_point
    } else {
        // LC_UNIXTHREAD: absolute address
        binary.entry_point
    };

    log::info!("binary mapped, entry point at {entry:#x}");
    Ok(entry)
}
