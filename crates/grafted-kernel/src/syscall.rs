//! Darwin BSD syscalls use the convention: rax = 0x2000000 | unix_syscall_nr.

/// Darwin syscall class bits (top byte of rax).
pub const DARWIN_SYSCALL_CLASS_MASK: u64 = 0xFF00_0000;
pub const DARWIN_SYSCALL_CLASS_UNIX: u64 = 0x0200_0000;
pub const DARWIN_SYSCALL_CLASS_MACH: u64 = 0x0100_0000;
pub const DARWIN_SYSCALL_NUMBER_MASK: u64 = 0x00FF_FFFF;

/// Result of classifying a Darwin syscall.
#[derive(Debug, Clone, Copy)]
pub enum DarwinSyscall {
    /// BSD/UNIX syscall - can be translated to Linux equivalent.
    Unix { darwin_nr: u32, linux_nr: i64 },
    /// Mach trap - needs kernel module handling.
    MachTrap { trap_nr: u32 },
    /// Unknown or unsupported syscall class.
    Unknown { raw: u64 },
}

/// Classify and translate a raw Darwin syscall number (from rax).
pub fn translate(raw_syscall: u64) -> DarwinSyscall {
    let class = raw_syscall & DARWIN_SYSCALL_CLASS_MASK;
    let nr = (raw_syscall & DARWIN_SYSCALL_NUMBER_MASK) as u32;

    match class {
        DARWIN_SYSCALL_CLASS_UNIX => {
            let linux_nr = darwin_unix_to_linux(nr);
            DarwinSyscall::Unix {
                darwin_nr: nr,
                linux_nr,
            }
        }
        DARWIN_SYSCALL_CLASS_MACH => DarwinSyscall::MachTrap { trap_nr: nr },
        _ => DarwinSyscall::Unknown { raw: raw_syscall },
    }
}

/// Map a Darwin BSD syscall number to a Linux syscall number.
/// Returns -1 for unimplemented syscalls.
fn darwin_unix_to_linux(darwin_nr: u32) -> i64 {
    // Darwin BSD -> Linux x86_64 syscall number mapping.
    match darwin_nr {
        // Process lifecycle
        1 => 60,    // exit -> exit
        2 => 57,    // fork -> fork
        7 => 61,    // wait4 -> wait4
        20 => 39,   // getpid -> getpid

        // File I/O
        3 => 0,     // read -> read
        4 => 1,     // write -> write
        5 => 2,     // open -> open
        6 => 3,     // close -> close

        // File operations
        10 => 87,   // unlink -> unlink
        15 => 90,   // chmod -> chmod
        16 => 93,   // chown -> chown

        // Memory
        197 => 9,   // mmap -> mmap
        73 => 11,   // munmap -> munmap
        74 => 10,   // mprotect -> mprotect

        // Signals
        46 => 13,   // sigaction -> rt_sigaction
        48 => 62,   // kill -> kill
        13 => 13,   // fchdir -> fchdir (also catches unprefixed rt_sigaction)

        // Directory
        59 => 84,   // execve -> ... (complex, not 1:1)
        338 => 59,  // posix_spawn -> (no direct equivalent, map to execve for now)

        // Time
        116 => 96,  // gettimeofday -> gettimeofday

        // Identity
        24 => 102,  // getuid -> getuid
        25 => 104,  // geteuid -> geteuid
        43 => 103,  // getegid -> getegid
        47 => 107,  // getgid -> getgid

        // Socket (numbers differ significantly)

        // ioctl
        54 => 16,   // ioctl -> ioctl

        // Unimplemented
        _ => -1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_translate_write() {
        match translate(0x2000004) {
            DarwinSyscall::Unix {
                darwin_nr: 4,
                linux_nr: 1,
            } => {}
            other => panic!("expected Unix write, got {other:?}"),
        }
    }

    #[test]
    fn test_translate_exit() {
        match translate(0x2000001) {
            DarwinSyscall::Unix {
                darwin_nr: 1,
                linux_nr: 60,
            } => {}
            other => panic!("expected Unix exit, got {other:?}"),
        }
    }

    #[test]
    fn test_translate_mach_trap() {
        match translate(0x100001a) {
            DarwinSyscall::MachTrap { trap_nr: 26 } => {}
            other => panic!("expected MachTrap 26, got {other:?}"),
        }
    }

    #[test]
    fn test_translate_unknown() {
        match translate(0x0500_0001) {
            DarwinSyscall::Unknown { .. } => {}
            other => panic!("expected Unknown, got {other:?}"),
        }
    }
}
