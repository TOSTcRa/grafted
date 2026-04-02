//! libSystem.B.dylib shim — in-process function table.
//!
//! Shim functions toggle the SUD selector byte (ALLOW before syscall, BLOCK after)
//! so that our Linux syscalls pass through while Darwin code remains intercepted.

use std::arch::asm;
use std::collections::HashMap;
use std::sync::atomic::{AtomicPtr, Ordering};

static SELECTOR_PTR: AtomicPtr<u8> = AtomicPtr::new(std::ptr::null_mut());

const FILTER_ALLOW: u8 = 0;
const FILTER_BLOCK: u8 = 1;

pub fn set_selector_ptr(ptr: *mut u8) {
    SELECTOR_PTR.store(ptr, Ordering::Release);
}

fn selector_allow() {
    let ptr = SELECTOR_PTR.load(Ordering::Acquire);
    if !ptr.is_null() {
        unsafe { ptr.write_volatile(FILTER_ALLOW) };
    }
}

fn selector_block() {
    let ptr = SELECTOR_PTR.load(Ordering::Acquire);
    if !ptr.is_null() {
        unsafe { ptr.write_volatile(FILTER_BLOCK) };
    }
}

pub fn default_registry() -> HashMap<String, HashMap<String, u64>> {
    let mut registry = HashMap::new();
    let mut s: HashMap<String, u64> = HashMap::new();

    s.insert("_write".into(), shim_write as *const () as u64);
    s.insert("_read".into(), shim_read as *const () as u64);
    s.insert("_open".into(), shim_open as *const () as u64);
    s.insert("_close".into(), shim_close as *const () as u64);
    s.insert("_exit".into(), shim_exit as *const () as u64);
    s.insert("__exit".into(), shim_exit as *const () as u64);
    s.insert("_mmap".into(), shim_mmap as *const () as u64);
    s.insert("_munmap".into(), shim_munmap as *const () as u64);
    s.insert("_getpid".into(), shim_getpid as *const () as u64);
    s.insert("dyld_stub_binder".into(), shim_dyld_stub_binder as *const () as u64);

    for name in [
        "/usr/lib/libSystem.B.dylib",
        "/usr/lib/libSystem.dylib",
        "libSystem.B.dylib",
        "libSystem.dylib",
    ] {
        registry.insert(name.into(), s.clone());
    }

    registry
}

macro_rules! linux_syscall {
    ($nr:expr, $a1:expr) => {{
        selector_allow();
        let ret: i64;
        unsafe {
            asm!(
                "syscall",
                inlateout("rax") $nr as i64 => ret,
                in("rdi") $a1 as u64,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack),
            );
        }
        selector_block();
        ret
    }};
    ($nr:expr, $a1:expr, $a2:expr, $a3:expr) => {{
        selector_allow();
        let ret: i64;
        unsafe {
            asm!(
                "syscall",
                inlateout("rax") $nr as i64 => ret,
                in("rdi") $a1 as u64,
                in("rsi") $a2 as u64,
                in("rdx") $a3 as u64,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack),
            );
        }
        selector_block();
        ret
    }};
    ($nr:expr, $a1:expr, $a2:expr, $a3:expr, $a4:expr, $a5:expr, $a6:expr) => {{
        selector_allow();
        let ret: i64;
        unsafe {
            asm!(
                "syscall",
                inlateout("rax") $nr as i64 => ret,
                in("rdi") $a1 as u64,
                in("rsi") $a2 as u64,
                in("rdx") $a3 as u64,
                in("r10") $a4 as u64,
                in("r8") $a5 as u64,
                in("r9") $a6 as u64,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack),
            );
        }
        selector_block();
        ret
    }};
}

unsafe extern "C" fn shim_write(fd: u64, buf: u64, count: u64) -> i64 {
    linux_syscall!(1, fd, buf, count)
}

unsafe extern "C" fn shim_read(fd: u64, buf: u64, count: u64) -> i64 {
    linux_syscall!(0, fd, buf, count)
}

unsafe extern "C" fn shim_open(path: u64, flags: u64, mode: u64) -> i64 {
    linux_syscall!(2, path, flags, mode)
}

unsafe extern "C" fn shim_close(fd: u64) -> i64 {
    linux_syscall!(3, fd)
}

unsafe extern "C" fn shim_exit(status: u64) -> ! {
    selector_allow();
    unsafe {
        asm!(
            "syscall",
            in("rax") 231_i64,
            in("rdi") status,
            options(noreturn, nostack),
        );
    }
}

unsafe extern "C" fn shim_mmap(addr: u64, len: u64, prot: u64, flags: u64, fd: u64, offset: u64) -> i64 {
    linux_syscall!(9, addr, len, prot, flags, fd, offset)
}

unsafe extern "C" fn shim_munmap(addr: u64, len: u64, _: u64) -> i64 {
    linux_syscall!(11, addr, len, 0)
}

unsafe extern "C" fn shim_getpid() -> i64 {
    linux_syscall!(39, 0)
}

unsafe extern "C" fn shim_dyld_stub_binder() -> i64 {
    0
}
