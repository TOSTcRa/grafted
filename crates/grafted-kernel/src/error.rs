use thiserror::Error;

#[derive(Debug, Error)]
pub enum KernelError {
    #[error("failed to open /dev/grafted: {0}")]
    DeviceOpen(std::io::Error),

    #[error("ioctl failed: {0}")]
    Ioctl(#[from] nix::Error),

    #[error("kernel module not loaded - run `sudo insmod grafted-kmod.ko`")]
    ModuleNotLoaded,
}
