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
        // LC_MAIN: offset relative to the file start
        let text_seg = binary
            .segments
            .iter()
            .find(|s| s.name == "__TEXT")
            .ok_or(LoaderError::NoTextSegment)?;
        let base_addr = text_seg.vmaddr - text_seg.fileoff;
        base_addr + binary.entry_point
    } else {
        // LC_UNIXTHREAD: absolute address
        binary.entry_point
    };

    log::info!("binary mapped, entry point at {entry:#x}");
    Ok(entry)
}
