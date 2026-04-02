//! Mach-O parsing and segment mapping.
//!
//! Uses goblin for header/load command parsing. Extracts segments, finds
//! the entry point (LC_MAIN or LC_UNIXTHREAD), and identifies dynamic
//! library dependencies (LC_LOAD_DYLIB).

use std::path::Path;

use goblin::mach::{
    MachO,
    header::{MH_EXECUTE, MH_DYLIB, MH_BUNDLE},
    load_command::CommandVariant,
};

use crate::error::LoaderError;

const CPU_TYPE_X86_64: u32 = 0x0100_0007;

#[derive(Debug, Clone)]
pub struct Segment {
    pub name: String,
    pub vmaddr: u64,
    pub vmsize: u64,
    pub fileoff: u64,
    pub filesize: u64,
    pub maxprot: u32,
    pub initprot: u32,
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
    pub data: Vec<u8>,
}

impl MachOBinary {
    /// Parse a Mach-O file from disk. For fat (universal) binaries,
    /// selects the x86_64 slice.
    pub fn from_path(path: &Path) -> Result<Self, LoaderError> {
        let data = std::fs::read(path)?;
        Self::parse(path.display().to_string(), data)
    }

    /// Parse Mach-O from raw bytes.
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

    /// Parse a single (non-fat) Mach-O binary.
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
            data,
        })
    }

    /// Parse a fat (universal) binary, selecting the x86_64 slice.
    fn parse_fat(path: String, data: Vec<u8>) -> Result<Self, LoaderError> {
        use goblin::mach::fat::{FatHeader, FatArch, SIZEOF_FAT_HEADER, SIZEOF_FAT_ARCH};

        let header = FatHeader::parse(&data)
            .map_err(|e| LoaderError::Parse(e.to_string()))?;

        for i in 0..header.nfat_arch {
            let offset = SIZEOF_FAT_HEADER + (i as usize) * SIZEOF_FAT_ARCH;
            let arch = FatArch::parse(&data, offset)
                .map_err(|e| LoaderError::Parse(e.to_string()))?;

            if arch.cputype() == CPU_TYPE_X86_64 {
                let slice_data = arch.slice(&data).to_vec();
                if slice_data.is_empty() {
                    return Err(LoaderError::Parse("invalid fat arch slice".into()));
                }
                return Self::parse_single(path, slice_data);
            }
        }

        Err(LoaderError::NoArchSlice("x86_64".into()))
    }

    /// Extract parsed info from a Mach-O buffer without consuming it.
    fn extract_info(data: &[u8]) -> Result<ParsedInfo, LoaderError> {
        let macho = MachO::parse(data, 0)
            .map_err(|e| LoaderError::Parse(e.to_string()))?;

        let file_type = macho.header.filetype;

        match file_type {
            MH_EXECUTE | MH_DYLIB | MH_BUNDLE => {}
            other => return Err(LoaderError::UnsupportedFileType(other)),
        }

        let cpu_type = macho.header.cputype();

        let segments: Vec<Segment> = macho
            .segments
            .iter()
            .map(|seg| {
                let name = seg.name().unwrap_or("???").to_string();
                Segment {
                    name,
                    vmaddr: seg.vmaddr,
                    vmsize: seg.vmsize,
                    fileoff: seg.fileoff,
                    filesize: seg.filesize,
                    maxprot: seg.maxprot,
                    initprot: seg.initprot,
                }
            })
            .collect();

        // LC_MAIN (offset) or LC_UNIXTHREAD (absolute)
        let mut entry_point = 0u64;
        let mut entry_is_offset = false;
        let mut found_entry = false;

        for lc in &macho.load_commands {
            match lc.command {
                CommandVariant::Main(main_cmd) => {
                    entry_point = main_cmd.entryoff;
                    entry_is_offset = true;
                    found_entry = true;
                    break;
                }
                CommandVariant::Unixthread(thread_cmd) => {
                    // instruction_pointer needs cputype to know register layout
                    entry_point = thread_cmd
                        .instruction_pointer(cpu_type)
                        .map_err(|e| LoaderError::Parse(e.to_string()))?;
                    entry_is_offset = false;
                    found_entry = true;
                }
                _ => {}
            }
        }

        if !found_entry && file_type == MH_EXECUTE {
            return Err(LoaderError::NoEntryPoint);
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

        Ok(ParsedInfo {
            file_type,
            cpu_type,
            entry_point,
            entry_is_offset,
            segments,
            dylib_deps,
        })
    }
}

/// Intermediate struct to carry parsed info across the borrow boundary.
struct ParsedInfo {
    file_type: u32,
    cpu_type: u32,
    entry_point: u64,
    entry_is_offset: bool,
    segments: Vec<Segment>,
    dylib_deps: Vec<DylibDep>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_not_macho() {
        let result = MachOBinary::parse("test".into(), vec![0x00, 0x01, 0x02, 0x03]);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_hello_fixture() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/hello_x86_64.macho");
        if !fixture.exists() {
            // Skip if fixture not generated yet
            return;
        }
        let binary = MachOBinary::from_path(&fixture).expect("should parse hello fixture");
        assert_eq!(binary.file_type, 0x2); // MH_EXECUTE
        assert_eq!(binary.cpu_type, CPU_TYPE_X86_64);
        assert_eq!(binary.entry_point, 0x100000000);
        assert!(!binary.entry_is_offset); // LC_UNIXTHREAD = absolute
        assert_eq!(binary.segments.len(), 2);
        assert_eq!(binary.segments[0].name, "__PAGEZERO");
        assert_eq!(binary.segments[1].name, "__TEXT");
        assert!(binary.dylib_deps.is_empty());
    }
}
