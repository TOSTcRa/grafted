//! Integration test — pings /dev/grafted and checks the module responds.
//! Requires the kernel module to be loaded.

use grafted_kernel::ioctl::GraftedDevice;

#[test]
fn test_ping_kernel_module() {
    let dev = match GraftedDevice::open() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("skipping: {e}");
            return;
        }
    };

    let version = dev.ping().expect("ping ioctl failed");
    // GRAFTED_VERSION = 0x00010000 (0.1.0)
    assert_eq!(version, 0x0001_0000, "unexpected module version: {version:#x}");
    println!("kernel module responded with version {version:#x}");
}
