//! ioctl definitions for communicating with /dev/grafted.
//!
//! These must match the definitions in grafted-kmod/c/grafted_ioctl.h.

use std::fs::{File, OpenOptions};
use std::os::unix::io::AsRawFd;

use crate::error::KernelError;

const GRAFTED_DEVICE: &str = "/dev/grafted";

// ioctl magic number — 'G' for Grafted
const GRAFTED_IOC_MAGIC: u8 = b'G';

// ioctl command numbers
const _GRAFTED_IOC_PING: u8 = 0;
const _GRAFTED_IOC_REGISTER: u8 = 1;
const _GRAFTED_IOC_UNREGISTER: u8 = 2;
const _GRAFTED_IOC_MACH_TRAP: u8 = 3;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct GraftedMachTrap {
    pub trap_number: u32,
    pub args: [u64; 6],
    pub result: i64,
}

nix::ioctl_read!(grafted_ping, GRAFTED_IOC_MAGIC, _GRAFTED_IOC_PING, u64);
nix::ioctl_write_ptr!(grafted_register, GRAFTED_IOC_MAGIC, _GRAFTED_IOC_REGISTER, u32);
nix::ioctl_none!(grafted_unregister, GRAFTED_IOC_MAGIC, _GRAFTED_IOC_UNREGISTER);
nix::ioctl_readwrite!(grafted_mach_trap, GRAFTED_IOC_MAGIC, _GRAFTED_IOC_MACH_TRAP, GraftedMachTrap);

pub struct GraftedDevice {
    file: File,
}

impl GraftedDevice {
    /// Open /dev/grafted. Returns ModuleNotLoaded if the device doesn't exist.
    pub fn open() -> Result<Self, KernelError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(GRAFTED_DEVICE)
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    KernelError::ModuleNotLoaded
                } else {
                    KernelError::DeviceOpen(e)
                }
            })?;
        Ok(Self { file })
    }

    /// Verify the kernel module is loaded and responding.
    pub fn ping(&self) -> Result<u64, KernelError> {
        let mut version: u64 = 0;
        unsafe {
            grafted_ping(self.file.as_raw_fd(), &mut version)?;
        }
        Ok(version)
    }

    /// Register current process as a Darwin emulation target.
    pub fn register(&self, pid: u32) -> Result<(), KernelError> {
        unsafe {
            grafted_register(self.file.as_raw_fd(), &pid as *const _)?;
        }
        Ok(())
    }

    /// Unregister current process.
    pub fn unregister(&self) -> Result<(), KernelError> {
        unsafe {
            grafted_unregister(self.file.as_raw_fd())?;
        }
        Ok(())
    }

    /// Forward a Mach trap via ioctl.
    pub fn mach_trap(&self, trap: &mut GraftedMachTrap) -> Result<(), KernelError> {
        unsafe {
            grafted_mach_trap(self.file.as_raw_fd(), trap)?;
        }
        Ok(())
    }
}
