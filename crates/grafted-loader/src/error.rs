use thiserror::Error;

#[derive(Debug, Error)]
pub enum LoaderError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("not a Mach-O binary")]
    NotMachO,

    #[error("unsupported Mach-O file type: {0:#x}")]
    UnsupportedFileType(u32),

    #[error("unsupported CPU type: {0:#x}")]
    UnsupportedCpuType(u32),

    #[error("no executable segment found")]
    NoTextSegment,

    #[error("no entry point found (missing LC_MAIN and LC_UNIXTHREAD)")]
    NoEntryPoint,

    #[error("Mach-O parse error: {0}")]
    Parse(String),

    #[error("memory mapping failed: {0}")]
    Mmap(String),

    #[error("fat binary has no slice for architecture {0}")]
    NoArchSlice(String),
}
