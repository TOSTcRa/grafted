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

nix::ioctl_read!(grafted_ping, GRAFTED_IOC_MAGIC, _GRAFTED_IOC_PING, u64);

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

    /// Ping the kernel module to verify it's alive.
    pub fn ping(&self) -> Result<u64, KernelError> {
        let mut version: u64 = 0;
        unsafe {
            grafted_ping(self.file.as_raw_fd(), &mut version)?;
        }
        Ok(version)
    }
}
