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

    macro_rules! reg {
        ($name:expr, $fn:expr) => { s.insert($name.into(), $fn as *const () as u64); };
    }

    // POSIX I/O
    reg!("_write", shim_write);
    reg!("_read", shim_read);
    reg!("_open", shim_open);
    reg!("_close", shim_close);
    reg!("_lseek", shim_lseek);
    reg!("_dup", shim_dup);
    reg!("_dup2", shim_dup2);
    reg!("_fcntl", shim_fcntl);
    reg!("_fstat", shim_fstat);
    reg!("_stat", shim_stat);
    reg!("_isatty", shim_isatty);

    // Process
    reg!("_exit", shim_exit);
    reg!("__exit", shim_exit);
    reg!("_getpid", shim_getpid);
    reg!("_getuid", shim_getuid);
    reg!("_geteuid", shim_geteuid);
    reg!("_getgid", shim_getgid);
    reg!("_getegid", shim_getegid);

    // Memory
    reg!("_mmap", shim_mmap);
    reg!("_munmap", shim_munmap);
    reg!("_mprotect", shim_mprotect);
    reg!("_malloc", shim_malloc);
    reg!("_free", shim_free);
    reg!("_calloc", shim_calloc);
    reg!("_realloc", shim_realloc);

    // String/memory (pure computation, no syscalls)
    reg!("_strlen", shim_strlen);
    reg!("_memcpy", shim_memcpy);
    reg!("_memmove", shim_memmove);
    reg!("_memset", shim_memset);
    reg!("_memcmp", shim_memcmp);
    reg!("_strcmp", shim_strcmp);
    reg!("_strncmp", shim_strncmp);
    reg!("_strcpy", shim_strcpy);
    reg!("_strncpy", shim_strncpy);
    reg!("_bzero", shim_bzero);

    // stdio-like
    reg!("_puts", shim_puts);

    // dyld
    reg!("dyld_stub_binder", shim_dyld_stub_binder);
    reg!("___stack_chk_fail", shim_stack_chk_fail);
    reg!("___stack_chk_guard", shim_stack_chk_guard_addr);

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

// ---- Syscall wrappers ----

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
    ($nr:expr, $a1:expr, $a2:expr) => {{
        selector_allow();
        let ret: i64;
        unsafe {
            asm!(
                "syscall",
                inlateout("rax") $nr as i64 => ret,
                in("rdi") $a1 as u64,
                in("rsi") $a2 as u64,
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

// ---- I/O ----

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
unsafe extern "C" fn shim_lseek(fd: u64, offset: u64, whence: u64) -> i64 {
    linux_syscall!(8, fd, offset, whence)
}
unsafe extern "C" fn shim_dup(fd: u64) -> i64 {
    linux_syscall!(32, fd)
}
unsafe extern "C" fn shim_dup2(old: u64, new: u64) -> i64 {
    linux_syscall!(33, old, new)
}
unsafe extern "C" fn shim_fcntl(fd: u64, cmd: u64, arg: u64) -> i64 {
    linux_syscall!(72, fd, cmd, arg)
}
unsafe extern "C" fn shim_fstat(fd: u64, buf: u64) -> i64 {
    linux_syscall!(5, fd, buf)
}
unsafe extern "C" fn shim_stat(path: u64, buf: u64) -> i64 {
    linux_syscall!(4, path, buf)
}
unsafe extern "C" fn shim_isatty(fd: u64) -> i64 {
    // ioctl(fd, TCGETS, &termios) — if it succeeds, fd is a tty
    let mut termios = [0u8; 256];
    let ret = linux_syscall!(16, fd, 0x5401u64, termios.as_mut_ptr() as u64);
    if ret == 0 { 1 } else { 0 }
}

// ---- Process ----

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
unsafe extern "C" fn shim_getpid() -> i64 { linux_syscall!(39, 0) }
unsafe extern "C" fn shim_getuid() -> i64 { linux_syscall!(102, 0) }
unsafe extern "C" fn shim_geteuid() -> i64 { linux_syscall!(107, 0) }
unsafe extern "C" fn shim_getgid() -> i64 { linux_syscall!(104, 0) }
unsafe extern "C" fn shim_getegid() -> i64 { linux_syscall!(108, 0) }

// ---- Memory (syscalls) ----

unsafe extern "C" fn shim_mmap(addr: u64, len: u64, prot: u64, flags: u64, fd: u64, off: u64) -> i64 {
    linux_syscall!(9, addr, len, prot, flags, fd, off)
}
unsafe extern "C" fn shim_munmap(addr: u64, len: u64) -> i64 {
    linux_syscall!(11, addr, len)
}
unsafe extern "C" fn shim_mprotect(addr: u64, len: u64, prot: u64) -> i64 {
    linux_syscall!(10, addr, len, prot)
}

// ---- malloc/free via mmap ----
// Simple bump allocator: each malloc does an mmap, free does munmap.
// Not efficient, but correct. Real binaries will need a proper allocator later.

unsafe extern "C" fn shim_malloc(size: u64) -> u64 {
    if size == 0 { return 0; }
    // mmap(NULL, size+16, PROT_READ|PROT_WRITE, MAP_PRIVATE|MAP_ANONYMOUS, -1, 0)
    // Store the allocation size in the first 8 bytes, return ptr+16 (aligned)
    let alloc_size = size + 16;
    let ret = linux_syscall!(9, 0u64, alloc_size, 3u64, 0x22u64, (-1i64) as u64, 0u64);
    if ret < 0 { return 0; }
    let base = ret as *mut u64;
    unsafe { base.write(alloc_size) };
    (ret + 16) as u64
}

unsafe extern "C" fn shim_free(ptr: u64) {
    if ptr == 0 { return; }
    let base = (ptr - 16) as *mut u64;
    let size = unsafe { base.read() };
    linux_syscall!(11, base as u64, size);
}

unsafe extern "C" fn shim_calloc(count: u64, size: u64) -> u64 {
    let total = count.wrapping_mul(size);
    unsafe { shim_malloc(total) }
}

unsafe extern "C" fn shim_realloc(ptr: u64, new_size: u64) -> u64 {
    if ptr == 0 { return unsafe { shim_malloc(new_size) }; }
    if new_size == 0 { unsafe { shim_free(ptr) }; return 0; }
    let old_base = (ptr - 16) as *mut u64;
    let old_size = unsafe { old_base.read() } - 16;
    let new_ptr = unsafe { shim_malloc(new_size) };
    if new_ptr == 0 { return 0; }
    let copy_size = if old_size < new_size { old_size } else { new_size };
    unsafe {
        std::ptr::copy_nonoverlapping(ptr as *const u8, new_ptr as *mut u8, copy_size as usize);
        shim_free(ptr);
    }
    new_ptr
}

// ---- String/memory (pure, no syscalls needed) ----

unsafe extern "C" fn shim_strlen(s: u64) -> u64 {
    let mut p = s as *const u8;
    let mut len = 0u64;
    unsafe { while p.read() != 0 { p = p.add(1); len += 1; } }
    len
}

unsafe extern "C" fn shim_memcpy(dst: u64, src: u64, n: u64) -> u64 {
    unsafe { std::ptr::copy_nonoverlapping(src as *const u8, dst as *mut u8, n as usize) };
    dst
}

unsafe extern "C" fn shim_memmove(dst: u64, src: u64, n: u64) -> u64 {
    unsafe { std::ptr::copy(src as *const u8, dst as *mut u8, n as usize) };
    dst
}

unsafe extern "C" fn shim_memset(dst: u64, val: u64, n: u64) -> u64 {
    unsafe { std::ptr::write_bytes(dst as *mut u8, val as u8, n as usize) };
    dst
}

unsafe extern "C" fn shim_memcmp(a: u64, b: u64, n: u64) -> i64 {
    let a = a as *const u8;
    let b = b as *const u8;
    for i in 0..n as usize {
        let diff = unsafe { *a.add(i) as i64 - *b.add(i) as i64 };
        if diff != 0 { return diff; }
    }
    0
}

unsafe extern "C" fn shim_strcmp(a: u64, b: u64) -> i64 {
    let mut a = a as *const u8;
    let mut b = b as *const u8;
    unsafe {
        loop {
            let ca = *a;
            let cb = *b;
            if ca != cb || ca == 0 { return ca as i64 - cb as i64; }
            a = a.add(1);
            b = b.add(1);
        }
    }
}

unsafe extern "C" fn shim_strncmp(a: u64, b: u64, n: u64) -> i64 {
    let mut a = a as *const u8;
    let mut b = b as *const u8;
    unsafe {
        for _ in 0..n {
            let ca = *a;
            let cb = *b;
            if ca != cb || ca == 0 { return ca as i64 - cb as i64; }
            a = a.add(1);
            b = b.add(1);
        }
    }
    0
}

unsafe extern "C" fn shim_strcpy(dst: u64, src: u64) -> u64 {
    let mut d = dst as *mut u8;
    let mut s = src as *const u8;
    unsafe {
        loop {
            let c = *s;
            *d = c;
            if c == 0 { break; }
            d = d.add(1);
            s = s.add(1);
        }
    }
    dst
}

unsafe extern "C" fn shim_strncpy(dst: u64, src: u64, n: u64) -> u64 {
    let d = dst as *mut u8;
    let s = src as *const u8;
    let mut i = 0usize;
    unsafe {
        while i < n as usize {
            let c = *s.add(i);
            *d.add(i) = c;
            if c == 0 { break; }
            i += 1;
        }
        while i < n as usize {
            *d.add(i) = 0;
            i += 1;
        }
    }
    dst
}

unsafe extern "C" fn shim_bzero(dst: u64, n: u64) {
    unsafe { std::ptr::write_bytes(dst as *mut u8, 0, n as usize) };
}

// ---- stdio-like ----

unsafe extern "C" fn shim_puts(s: u64) -> i64 {
    let len = unsafe { shim_strlen(s) };
    unsafe { shim_write(1, s, len) };
    unsafe { shim_write(1, b"\n".as_ptr() as u64, 1) };
    0
}

// ---- dyld / security ----

unsafe extern "C" fn shim_dyld_stub_binder() -> i64 { 0 }

unsafe extern "C" fn shim_stack_chk_fail() -> ! {
    let msg = b"*** stack smashing detected ***\n";
    selector_allow();
    unsafe {
        asm!(
            "syscall",
            in("rax") 1_i64,
            in("rdi") 2_u64,
            in("rsi") msg.as_ptr() as u64,
            in("rdx") msg.len() as u64,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
        asm!(
            "syscall",
            in("rax") 231_i64,
            in("rdi") 6_u64, // SIGABRT
            options(noreturn, nostack),
        );
    }
}

// Stack canary value — just needs to be non-zero and consistent
static STACK_CHK_GUARD: u64 = 0x00000aff0a0d0000;
unsafe extern "C" fn shim_stack_chk_guard_addr() -> u64 {
    (&raw const STACK_CHK_GUARD) as u64
}
