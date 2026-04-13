use std::path::Path;

use goblin::mach::load_command::CommandVariant;
use goblin::mach::Mach;

use crate::error::LoaderError;

pub const CPU_TYPE_X86_64: u32 = 0x01000007;

#[derive(Debug, Clone)]
pub struct Segment {
    pub name: String,
    pub vmaddr: u64,
    pub vmsize: u64,
    pub fileoff: u64,
    pub filesize: u64,
    pub initprot: u32,
    pub maxprot: u32,
}

#[derive(Debug, Clone)]
pub struct DylibDep {
    pub name: String,
    pub current_version: u32,
    pub compat_version: u32,
}

#[derive(Debug)]
pub struct MachOBinary {
    pub path: String,
    pub file_type: u32,
    pub cpu_type: u32,
    /// Entry point offset from __TEXT base (from LC_MAIN) or absolute address (LC_UNIXTHREAD).
    pub entry_point: u64,
    /// Whether entry_point is relative to __TEXT (LC_MAIN) or absolute (LC_UNIXTHREAD).
    pub entry_is_offset: bool,
    pub segments: Vec<Segment>,
    pub dylib_deps: Vec<DylibDep>,
    pub rpaths: Vec<String>,
    pub chained_fixups: Option<(u32, u32)>, // (offset, size)
    pub exports_trie: Option<(u32, u32)>,   // (offset, size)
    pub data: Vec<u8>,
    pub slide: u64,
}

impl MachOBinary {
    /// Fat binaries are narrowed to the x86_64 slice.
    pub fn from_path(path: &Path) -> Result<Self, LoaderError> {
        let data = std::fs::read(path)?;
        Self::parse(path.display().to_string(), data)
    }

    pub fn parse(path: String, data: Vec<u8>) -> Result<Self, LoaderError> {
        let is_fat = data.len() >= 4 && {
            let magic = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
            magic == 0xcafe_babe || magic == 0xcafe_babf
        };

        if is_fat {
            Self::parse_fat(path, data)
        } else {
            Self::parse_single(path, data)
        }
    }

    fn parse_single(path: String, data: Vec<u8>) -> Result<Self, LoaderError> {
        let info = Self::extract_info(&data)?;
        Ok(MachOBinary {
            path,
            file_type: info.file_type,
            cpu_type: info.cpu_type,
            entry_point: info.entry_point,
            entry_is_offset: info.entry_is_offset,
            segments: info.segments,
            dylib_deps: info.dylib_deps,
            rpaths: info.rpaths,
            chained_fixups: info.chained_fixups,
            exports_trie: info.exports_trie,
            data,
            slide: 0,
        })
    }

    fn parse_fat(path: String, data: Vec<u8>) -> Result<Self, LoaderError> {
        let mach = Mach::parse(&data).map_err(|e| LoaderError::Parse(e.to_string()))?;
        match mach {
            Mach::Fat(fat) => {
                for arch in fat.arches().map_err(|e| LoaderError::Parse(e.to_string()))? {
                    if arch.cputype == CPU_TYPE_X86_64 {
                        let offset = arch.offset as usize;
                        let size = arch.size as usize;
                        let slice = data[offset..offset + size].to_vec();
                        return Self::parse_single(path, slice);
                    }
                }
                Err(LoaderError::UnsupportedCpuType(0))
            }
            Mach::Binary(macho) => {
                // Should not happen if is_fat was true, but handle anyway
                if macho.header.cputype == CPU_TYPE_X86_64 {
                    Self::parse_single(path, data)
                } else {
                    Err(LoaderError::UnsupportedCpuType(macho.header.cputype))
                }
            }
        }
    }

    /// Extract info from data without taking ownership.
    fn extract_info(data: &[u8]) -> Result<ParsedInfo, LoaderError> {
        let macho = goblin::mach::MachO::parse(data, 0)
            .map_err(|e| LoaderError::Parse(e.to_string()))?;

        let file_type = macho.header.filetype;
        let cpu_type = macho.header.cputype;

        if cpu_type != CPU_TYPE_X86_64 {
            return Err(LoaderError::UnsupportedCpuType(cpu_type));
        }

        let mut segments = Vec::new();
        let mut entry_point = 0;
        let mut entry_is_offset = false;
        let mut chained_fixups = None;
        let mut exports_trie = None;

        for lc in &macho.load_commands {
            match &lc.command {
                CommandVariant::Segment64(seg) => {
                    segments.push(Segment {
                        name: seg.name().unwrap_or("").to_string(),
                        vmaddr: seg.vmaddr,
                        vmsize: seg.vmsize,
                        fileoff: seg.fileoff,
                        filesize: seg.filesize,
                        initprot: seg.initprot,
                        maxprot: seg.maxprot,
                    });
                }
                CommandVariant::Main(main) => {
                    entry_point = main.entryoff;
                    entry_is_offset = true;
                }
                CommandVariant::Unixthread(thread) => {
                    if thread.flavor == 4 && thread.count >= 42 {
                        entry_point = ((thread.thread_state[33] as u64) << 32)
                            | (thread.thread_state[32] as u64);
                        entry_is_offset = false;
                    }
                }
                CommandVariant::DyldChainedFixups(fixups) => {
                    chained_fixups = Some((fixups.dataoff, fixups.datasize));
                }
                CommandVariant::DyldExportsTrie(trie) => {
                    exports_trie = Some((trie.dataoff, trie.datasize));
                }
                _ => {}
            }
        }

        let self_name = macho.name.unwrap_or("");
        let dylib_deps: Vec<DylibDep> = macho
            .libs
            .iter()
            .filter(|lib| !lib.is_empty() && **lib != self_name && **lib != "self")
            .map(|lib| DylibDep {
                name: lib.to_string(),
                current_version: 0,
                compat_version: 0,
            })
            .collect();

        let rpaths: Vec<String> = macho.rpaths.iter().map(|s| s.to_string()).collect();

        Ok(ParsedInfo {
            file_type,
            cpu_type,
            entry_point,
            entry_is_offset,
            segments,
            dylib_deps,
            rpaths,
            chained_fixups,
            exports_trie,
        })
    }
}

impl MachOBinary {
    /// Find __thread_data and __thread_bss sections for TLV initialization.
    /// Returns (thread_data_vmaddr, thread_data_size, total_tlv_size).
    pub fn tlv_init_image(&self) -> Option<(u64, usize, usize)> {
        use goblin::mach::MachO;
        let macho = MachO::parse(&self.data, 0).ok()?;
        let mut data_addr: u64 = 0;
        let mut data_size: u64 = 0;
        let mut bss_size: u64 = 0;
        for seg in &macho.segments {
            for section_res in seg {
                if let Ok((section, _)) = section_res {
                    match section.name().unwrap_or("") {
                        "__thread_data" => { data_addr = section.addr; data_size = section.size; }
                        "__thread_bss" => { bss_size = section.size; }
                        _ => {}
                    }
                }
            }
        }
        if data_size > 0 || bss_size > 0 {
            Some((data_addr, data_size as usize, (data_size + bss_size) as usize))
        } else {
            None
        }
    }
}

struct ParsedInfo {
    file_type: u32,
    cpu_type: u32,
    entry_point: u64,
    entry_is_offset: bool,
    segments: Vec<Segment>,
    dylib_deps: Vec<DylibDep>,
    rpaths: Vec<String>,
    chained_fixups: Option<(u32, u32)>,
    exports_trie: Option<(u32, u32)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_not_macho() {
        let result = MachOBinary::parse("test".into(), vec![0x00, 0x01, 0x02, 0x03]);
        assert!(result.is_err());
    }
}
