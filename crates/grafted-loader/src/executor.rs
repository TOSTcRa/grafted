//! Darwin binary executor.
//!
//! Uses Syscall User Dispatch (SUD, Linux 5.11+) to intercept Darwin syscalls.
//! When the Mach-O code executes a `syscall` instruction with Darwin calling
//! convention (rax = 0x2000000 | nr), SUD delivers SIGSYS. Our signal handler
//! translates the syscall to Linux and executes it.
//!
//! Key insight: after the handler returns, the kernel's sigreturn also makes
//! a syscall. We leave selector=ALLOW for sigreturn to succeed, and use a
//! trampoline that re-sets selector=BLOCK before resuming the Darwin code.

use std::arch::asm;
use std::cell::Cell;
use std::num::NonZeroUsize;

use nix::sys::mman::{mmap_anonymous, MapFlags, ProtFlags};

use grafted_kernel::syscall::{self, DarwinSyscall};

use crate::error::LoaderError;

// ---- SUD constants ----

const PR_SET_SYSCALL_USER_DISPATCH: libc::c_int = 59;
const PR_SYS_DISPATCH_ON: libc::c_int = 1;
pub const SYSCALL_DISPATCH_FILTER_ALLOW: u8 = 0;
pub const SYSCALL_DISPATCH_FILTER_BLOCK: u8 = 1;

use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

const SYS_USER_DISPATCH: i32 = 2;

// Per-thread SUD selectors: each thread gets its own byte from a shared page.
// This eliminates races when multiple threads toggle ALLOW/BLOCK concurrently.
static SELECTOR_PAGE: AtomicPtr<u8> = AtomicPtr::new(std::ptr::null_mut());
static SELECTOR_NEXT: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static THREAD_SELECTOR: Cell<*mut u8> = const { Cell::new(std::ptr::null_mut()) };
    static THREAD_TRAMPOLINE: Cell<u64> = const { Cell::new(0) };
}

static STACK_BASE_ADDR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static DARWIN_TEXT_BASE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn set_darwin_text_base(addr: u64) {
    DARWIN_TEXT_BASE.store(addr, std::sync::atomic::Ordering::Release);
}

pub fn stack_base() -> u64 {
    STACK_BASE_ADDR.load(std::sync::atomic::Ordering::Acquire)
}

/// Get (or lazily allocate) the shared selector page.
fn selector_page() -> *mut u8 {
    let ptr = SELECTOR_PAGE.load(Ordering::Acquire);
    if !ptr.is_null() { return ptr; }
    let size = NonZeroUsize::new(4096).unwrap();
    let new_ptr = unsafe {
        mmap_anonymous(
            None,
            size,
            ProtFlags::PROT_READ | ProtFlags::PROT_WRITE,
            MapFlags::MAP_PRIVATE,
        )
    }
    .expect("failed to allocate selector page")
    .as_ptr() as *mut u8;
    match SELECTOR_PAGE.compare_exchange(
        std::ptr::null_mut(),
        new_ptr,
        Ordering::Release,
        Ordering::Relaxed,
    ) {
        Ok(_) => new_ptr,
        Err(_) => SELECTOR_PAGE.load(Ordering::Acquire),
    }
}

/// Allocate a per-thread selector byte and store it in thread-local storage.
pub fn alloc_thread_selector() -> *mut u8 {
    let page = selector_page();
    let offset = SELECTOR_NEXT.fetch_add(1, Ordering::Relaxed);
    assert!(offset < 4096, "grafted: too many threads for selector page");
    let ptr = unsafe { page.add(offset) };
    unsafe { *ptr = SYSCALL_DISPATCH_FILTER_ALLOW };
    THREAD_SELECTOR.with(|c| c.set(ptr));
    ptr
}

/// Get the current thread's selector byte. Allocates one if not yet assigned.
pub fn selector_ptr() -> *mut u8 {
    let ptr = THREAD_SELECTOR.with(|c| c.get());
    if !ptr.is_null() { return ptr; }
    alloc_thread_selector()
}

/// Set up SUD for the current thread with a per-thread selector and trampoline.
pub fn setup_thread_sud(sel_ptr: *mut u8) {
    let tramp = alloc_trampoline_for(sel_ptr).expect("trampoline alloc");
    THREAD_SELECTOR.with(|c| c.set(sel_ptr));
    THREAD_TRAMPOLINE.with(|c| c.set(tramp));
    let text_base = DARWIN_TEXT_BASE.load(std::sync::atomic::Ordering::Acquire);
    let (sud_off, sud_len) = if text_base > 0 && text_base < 0x1_0000_0000 {
        (0_usize, 0_usize)
    } else {
        (0x1000_usize, 0xFFFF_F000_usize)
    };
    unsafe {
        libc::prctl(
            PR_SET_SYSCALL_USER_DISPATCH,
            PR_SYS_DISPATCH_ON,
            sud_off,
            sud_len,
            sel_ptr as usize,
        );
        *sel_ptr = SYSCALL_DISPATCH_FILTER_BLOCK;
    }
}

/// Called from gen_trampoline assembly to set the current thread's selector to ALLOW.
#[unsafe(no_mangle)]
pub extern "C" fn grafted_selector_allow() {
    let ptr = selector_ptr();
    unsafe { ptr.write_volatile(SYSCALL_DISPATCH_FILTER_ALLOW) };
}

static mut GRAFTED_FD: libc::c_int = -1;

// ---- Stack allocation ----

pub const STACK_SIZE: usize = 8 * 1024 * 1024;

// Returns (stack_base, stack_top)
fn alloc_stack() -> Result<(usize, usize), LoaderError> {
    let size = NonZeroUsize::new(STACK_SIZE).unwrap();
    let base = unsafe {
        mmap_anonymous(
            None,
            size,
            ProtFlags::PROT_READ | ProtFlags::PROT_WRITE,
            MapFlags::MAP_PRIVATE,
        )
    }
    .map_err(|e| LoaderError::Mmap(format!("stack alloc: {e}")))?;

    let base_addr = base.as_ptr() as usize;
    let top = base_addr + STACK_SIZE;
    Ok((base_addr, top & !0xF))
}

// ---- Trampoline ----

/// Allocate an executable trampoline that sets the given selector byte to BLOCK,
/// then returns to the real RIP (pushed on stack by the signal handler).
/// Each thread gets its own trampoline pointing to its own selector byte.
fn alloc_trampoline_for(sel_ptr: *mut u8) -> Result<u64, LoaderError> {
    let sel_addr = sel_ptr as u64;

    //   movabs r11, <sel_ptr>           ; 10 bytes (49 bb ...)
    //   mov byte ptr [r11], 1           ; 4 bytes (41 c6 03 01)
    //   ret                             ; 1 byte
    // Total: 15 bytes
    let mut code = [0u8; 15];
    code[0] = 0x49;
    code[1] = 0xbb;
    code[2..10].copy_from_slice(&sel_addr.to_le_bytes());
    code[10] = 0x41;
    code[11] = 0xc6;
    code[12] = 0x03;
    code[13] = SYSCALL_DISPATCH_FILTER_BLOCK;
    code[14] = 0xc3; // ret

    let size = NonZeroUsize::new(4096).unwrap();
    let page = unsafe {
        mmap_anonymous(
            None,
            size,
            ProtFlags::PROT_READ | ProtFlags::PROT_WRITE | ProtFlags::PROT_EXEC,
            MapFlags::MAP_PRIVATE,
        )
    }
    .map_err(|e| LoaderError::Mmap(format!("trampoline alloc: {e}")))?;

    unsafe {
        std::ptr::copy_nonoverlapping(code.as_ptr(), page.as_ptr() as *mut u8, code.len());
    }

    let addr = page.as_ptr() as u64;
    log::debug!("trampoline at {addr:#x} for selector {sel_addr:#x}");
    Ok(addr)
}

// ---- Syscall Processing ----

unsafe fn process_syscall(
    raw_syscall: u64,
    args: [u64; 6],
    fd: libc::c_int,
) -> i64 {
    match syscall::translate(raw_syscall) {
        DarwinSyscall::Unix { linux_nr, darwin_nr } if linux_nr >= 0 => {
            if linux_nr == 60 || linux_nr == 231 {
                unsafe {
                    asm!(
                        "syscall",
                        in("rax") 231_i64,
                        in("rdi") args[0],
                        options(noreturn, nostack),
                    );
                }
            }

            let mut a = args;
            match darwin_nr {
                // mmap: translate flags (Darwin MAP_ANON=0x1000 → Linux=0x20)
                197 | 199 => {
                    let df = a[3] as i32;
                    let mut lf = df & 0x1F;
                    if df & 0x1000 != 0 { lf |= 0x20; }
                    if df & 0x0040 != 0 { lf |= 0x4000; }
                    a[3] = lf as u64;
                    // Guard page (PROT_NONE + MAP_FIXED + MAP_ANON) → fake success
                    if a[2] == 0 && lf & 0x30 == 0x30 {
                        return if a[0] != 0 { a[0] as i64 } else { 0x7fff_0000 };
                    }
                }
                // mprotect: guard page (PROT_NONE) → fake success
                74 => {
                    if a[2] == 0 { return 0; }
                }
                // open: translate flags (Darwin O_CREAT=0x200 → Linux=0x40)
                5 => {
                    let df = a[1] as i32;
                    let mut lf = df & 0x3;
                    if df & 0x0008 != 0 { lf |= 0x0400; }
                    if df & 0x0004 != 0 { lf |= 0x0800; }
                    if df & 0x0200 != 0 { lf |= 0x0040; }
                    if df & 0x0400 != 0 { lf |= 0x0200; }
                    if df & 0x0800 != 0 { lf |= 0x0080; }
                    a[1] = lf as u64;
                }
                _ => {}
            }

            let ret: i64;
            unsafe {
                asm!(
                    "syscall",
                    inlateout("rax") linux_nr as i64 => ret,
                    in("rdi") a[0],
                    in("rsi") a[1],
                    in("rdx") a[2],
                    in("r10") a[3],
                    in("r8") a[4],
                    in("r9") a[5],
                    lateout("rcx") _,
                    lateout("r11") _,
                    options(nostack),
                );
            }
            ret
        }
        DarwinSyscall::Unix { .. } => -38,
        DarwinSyscall::MachTrap { trap_nr } => {
            match trap_nr {
                26 => 0x307,  // mach_reply_port
                27 => 0x203,  // mach_thread_self
                28 => 0x103,  // mach_task_self
                // mach_vm_protect → mprotect (guard page PROT_NONE → fake success)
                14 => {
                    let prot = args[4] as i32;
                    if prot == 0 { return 0; }
                    let addr = args[1] & !0xFFF;
                    let size = ((args[1] + args[2] + 0xFFF) & !0xFFF) - addr;
                    let ret = unsafe { libc::mprotect(addr as *mut _, size as usize, prot) };
                    if ret == 0 { 0 } else { 1 }
                }
                _ => {
                    if fd < 0 { return -38; }
                    let mut trap = grafted_kernel::ioctl::GraftedMachTrap {
                        trap_number: trap_nr,
                        args,
                        result: 0,
                    };
                    let res = unsafe {
                        grafted_kernel::ioctl::grafted_mach_trap(fd, &mut trap)
                    };
                    if res.is_err() { -38 } else { trap.result }
                }
            }
        }
        DarwinSyscall::Unknown { raw } => {
            if raw < 500 {
                // Allow raw Linux syscalls to pass through unmodified.
                // This is crucial for our Linux libc shims to work.
                let ret: i64;
                unsafe {
                    asm!(
                        "syscall",
                        inlateout("rax") raw as i64 => ret,
                        in("rdi") args[0],
                        in("rsi") args[1],
                        in("rdx") args[2],
                        in("r10") args[3],
                        in("r8") args[4],
                        in("r9") args[5],
                        lateout("rcx") _,
                        lateout("r11") _,
                        options(nostack),
                    );
                }
                ret
            } else {
                let msg = b"grafted: unknown syscall class\n";
                unsafe { libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len()) };
                -38
            }
        }
    }
}

// ---- SIGSYS signal handler ----

unsafe extern "C" fn sigsys_handler(
    _sig: libc::c_int,
    info: *mut libc::siginfo_t,
    context: *mut libc::c_void,
) {
    // Per-thread selector: each thread has its own byte, no races.
    let sel_ptr = THREAD_SELECTOR.with(|c| c.get());
    if sel_ptr.is_null() { return; }

    // FIRST: allow our own syscalls
    unsafe { *sel_ptr = SYSCALL_DISPATCH_FILTER_ALLOW };

    let info = unsafe { &*info };
    if info.si_code != SYS_USER_DISPATCH {
        unsafe { *sel_ptr = SYSCALL_DISPATCH_FILTER_BLOCK };
        return;
    }

    let uctx = unsafe { &mut *(context as *mut libc::ucontext_t) };
    let gregs = &mut uctx.uc_mcontext.gregs;

    let raw_syscall = gregs[libc::REG_RAX as usize] as u64;

    let args = [
        gregs[libc::REG_RDI as usize] as u64,
        gregs[libc::REG_RSI as usize] as u64,
        gregs[libc::REG_RDX as usize] as u64,
        gregs[libc::REG_R10 as usize] as u64,
        gregs[libc::REG_R8 as usize] as u64,
        gregs[libc::REG_R9 as usize] as u64,
    ];

    let result = unsafe { process_syscall(raw_syscall, args, GRAFTED_FD) };

    gregs[libc::REG_RAX as usize] = result;

    // Push the real resume RIP onto the Darwin stack, then redirect to trampoline.
    // The per-thread trampoline sets THIS thread's SELECTOR=BLOCK then `ret`s.
    let trampoline = THREAD_TRAMPOLINE.with(|c| c.get());

    let real_rip = gregs[libc::REG_RIP as usize] as u64;
    let rsp = gregs[libc::REG_RSP as usize] as u64;
    let new_rsp = rsp - 8;
    unsafe { (new_rsp as *mut u64).write(real_rip) };
    gregs[libc::REG_RSP as usize] = new_rsp as i64;
    gregs[libc::REG_RIP as usize] = trampoline as i64;

    // Leave selector as ALLOW — sigreturn needs it to work.
    // The trampoline will set it back to BLOCK.
}

// ---- SUD setup ----

fn enable_sud() -> Result<(), LoaderError> {
    let sel_ptr = selector_ptr();
    // Dynamic range: Go binaries mapped at <4GB need empty range.
    // Rust binaries at >4GB use the standard range (our code + libc exempt).
    let text_base = DARWIN_TEXT_BASE.load(std::sync::atomic::Ordering::Acquire);
    let (sud_off, sud_len) = if text_base > 0 && text_base < 0x1_0000_0000 {
        (0_usize, 0_usize)
    } else {
        (0x1000_usize, 0xFFFF_F000_usize)
    };
    let ret = unsafe {
        libc::prctl(
            PR_SET_SYSCALL_USER_DISPATCH,
            PR_SYS_DISPATCH_ON,
            sud_off,
            sud_len,
            sel_ptr as usize,
        )
    };
    if ret != 0 {
        return Err(LoaderError::Mmap(format!(
            "prctl(PR_SET_SYSCALL_USER_DISPATCH) failed: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
}

// Recovery point for graceful SIGSEGV handling during lifecycle calls.
static mut RECOVERY_BUF: [u8; 256] = [0u8; 256]; // sigjmp_buf
static RECOVERY_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

unsafe extern "C" {
    fn __sigsetjmp(buf: *mut u8, save_sigs: libc::c_int) -> libc::c_int;
    fn siglongjmp(buf: *mut u8, val: libc::c_int) -> !;
}

/// Try to call a function, recovering from SIGSEGV if it crashes.
/// Returns true if the call succeeded, false if it crashed.
#[unsafe(no_mangle)]
pub extern "C" fn grafted_try_call(func: unsafe extern "C" fn(*mut u8, *mut u8), a: *mut u8, b: *mut u8) -> bool {
    unsafe {
        RECOVERY_ACTIVE.store(true, std::sync::atomic::Ordering::SeqCst);
        let ret = __sigsetjmp(std::ptr::addr_of_mut!(RECOVERY_BUF) as *mut u8, 1);
        if ret == 0 {
            func(a, b);
            RECOVERY_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
            true
        } else {
            // Recovered from SIGSEGV
            RECOVERY_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
            false
        }
    }
}

unsafe extern "C" fn sigsegv_handler(
    _sig: libc::c_int,
    info: *mut libc::siginfo_t,
    context: *mut libc::c_void,
) {
    // If recovery is active, longjmp back to the setjmp point
    if RECOVERY_ACTIVE.load(std::sync::atomic::Ordering::SeqCst) {
        unsafe { siglongjmp(std::ptr::addr_of_mut!(RECOVERY_BUF) as *mut u8, 1) };
    }

    let info = unsafe { &*info };
    let uctx = unsafe { &*(context as *const libc::ucontext_t) };
    let rip = uctx.uc_mcontext.gregs[libc::REG_RIP as usize] as u64;
    let rsp = uctx.uc_mcontext.gregs[libc::REG_RSP as usize] as u64;
    let rax = uctx.uc_mcontext.gregs[libc::REG_RAX as usize] as u64;
    let rdx = uctx.uc_mcontext.gregs[libc::REG_RDX as usize] as u64;
    let rdi = uctx.uc_mcontext.gregs[libc::REG_RDI as usize] as u64;
    let rsi = uctx.uc_mcontext.gregs[libc::REG_RSI as usize] as u64;
    let fault_addr = unsafe { info.si_addr() } as u64;

    // Print crash info with all relevant registers
    let mut buf = [0u8; 512];
    let mut p = 0;
    fn hex(val: u64, out: &mut [u8]) { let d = b"0123456789abcdef"; let mut v = val; for i in (0..16).rev() { out[i] = d[(v & 0xf) as usize]; v >>= 4; } }
    fn w(b: &mut [u8], p: &mut usize, s: &[u8]) { b[*p..*p+s.len()].copy_from_slice(s); *p += s.len(); }
    fn wh(b: &mut [u8], p: &mut usize, v: u64) { hex(v, &mut b[*p..*p+16]); *p += 16; }
    w(&mut buf, &mut p, b"SIGSEGV at=0x"); wh(&mut buf, &mut p, fault_addr);
    w(&mut buf, &mut p, b" rip=0x"); wh(&mut buf, &mut p, rip);
    w(&mut buf, &mut p, b"\n  rax=0x"); wh(&mut buf, &mut p, rax);
    w(&mut buf, &mut p, b" rdx=0x"); wh(&mut buf, &mut p, rdx);
    w(&mut buf, &mut p, b" rdi=0x"); wh(&mut buf, &mut p, rdi);
    w(&mut buf, &mut p, b" rsi=0x"); wh(&mut buf, &mut p, rsi);
    w(&mut buf, &mut p, b"\n  rsp=0x"); wh(&mut buf, &mut p, rsp);
    // Dump memory at rdi to debug metadata issues
    if rdi > 0x10000 && rdi < 0x7FFF_FFFF_FFFF {
        w(&mut buf, &mut p, b"\n  *rdi:");
        for i in 0..10u64 {
            let val = unsafe { *((rdi + i * 8) as *const u64) };
            if i == 4 { w(&mut buf, &mut p, b"\n      "); }
            w(&mut buf, &mut p, b" "); wh(&mut buf, &mut p, val);
        }
    }
    // Try dladdr to identify the crashing library/function
    let mut dl_info: libc::Dl_info = unsafe { std::mem::zeroed() };
    if unsafe { libc::dladdr(rip as *const _, &mut dl_info) } != 0 {
        w(&mut buf, &mut p, b"\n  base=0x"); wh(&mut buf, &mut p, dl_info.dli_fbase as u64);
        w(&mut buf, &mut p, b" off=0x"); wh(&mut buf, &mut p, rip - dl_info.dli_fbase as u64);
        w(&mut buf, &mut p, b"\n  lib=");
        if !dl_info.dli_fname.is_null() {
            let lib = unsafe { std::ffi::CStr::from_ptr(dl_info.dli_fname) };
            let bytes = lib.to_bytes();
            let len = bytes.len().min(120);
            w(&mut buf, &mut p, &bytes[..len]);
        }
        w(&mut buf, &mut p, b" sym=");
        if !dl_info.dli_sname.is_null() {
            let sym = unsafe { std::ffi::CStr::from_ptr(dl_info.dli_sname) };
            let bytes = sym.to_bytes();
            let len = bytes.len().min(120);
            w(&mut buf, &mut p, &bytes[..len]);
        }
    }
    buf[p] = b'\n'; p += 1;
    unsafe { libc::write(2, buf.as_ptr() as *const _, p) };
    unsafe { libc::_exit(139) };
}

fn install_sigsys_handler() -> Result<(), LoaderError> {
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = sigsys_handler as *const () as usize;
        sa.sa_flags = libc::SA_SIGINFO;
        libc::sigemptyset(&mut sa.sa_mask);
        let ret = libc::sigaction(libc::SIGSYS, &sa, std::ptr::null_mut());
        if ret != 0 {
            return Err(LoaderError::Mmap(format!(
                "sigaction(SIGSYS) failed: {}",
                std::io::Error::last_os_error()
            )));
        }

        // Install SIGSEGV handler for crash diagnostics
        let mut sa2: libc::sigaction = std::mem::zeroed();
        sa2.sa_sigaction = sigsegv_handler as *const () as usize;
        sa2.sa_flags = libc::SA_SIGINFO;
        libc::sigemptyset(&mut sa2.sa_mask);
        libc::sigaction(libc::SIGSEGV, &sa2, std::ptr::null_mut());
    }
    Ok(())
}

// ---- Go runtime patching ----

/// Patch Go's `runtime.settls` to call arch_prctl(ARCH_SET_GS) on Linux.
pub fn patch_go_settls(binary: &crate::macho::MachOBinary) {
    use goblin::mach::MachO;
    let Ok(macho) = MachO::parse(&binary.data, 0) else { return };
    let mut settls_addr: Option<u64> = None;
    for sym in macho.symbols() {
        if let Ok((name, nlist)) = sym {
            if (name == "_runtime.settls.abi0" || name == "_runtime.settls") && nlist.n_value != 0 {
                settls_addr = Some(nlist.n_value);
                break;
            }
        }
    }
    let Some(addr) = settls_addr else { return };
    log::info!("patching Go runtime.settls at {addr:#x}");
    // lea rsi,[rdi-0x30]; mov edi,0x1001; mov eax,158; syscall; ret
    let code: [u8; 17] = [
        0x48, 0x8d, 0x77, 0xd0, 0xbf, 0x01, 0x10, 0x00, 0x00,
        0xb8, 0x9e, 0x00, 0x00, 0x00, 0x0f, 0x05, 0xc3,
    ];
    let page = (addr & !0xFFF) as *mut libc::c_void;
    unsafe {
        libc::mprotect(page, 4096, libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC);
        std::ptr::copy_nonoverlapping(code.as_ptr(), addr as *mut u8, code.len());
        libc::mprotect(page, 4096, libc::PROT_READ | libc::PROT_EXEC);
    }
    log::info!("Go runtime.settls patched — arch_prctl(ARCH_SET_GS)");
}

// ---- Stack layout for LC_MAIN ----

/// Build a Darwin-compatible process stack with argc/argv/envp/apple[].
/// Returns the address of argc (where rsp should point on entry).
fn build_stack(
    stack_top: usize,
    argv: &[String],
    binary_path: &str,
) -> usize {
    let mut sp = stack_top;

    // Collect envp from host
    let envp: Vec<String> = std::env::vars()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();

    // apple[] entries (Darwin-specific metadata)
    let apple = vec![
        format!("executable_path={binary_path}"),
    ];

    let push_string = |sp: &mut usize, s: &str| -> u64 {
        let bytes = s.as_bytes();
        let len = bytes.len() + 1; // include null terminator
        *sp -= len;
        *sp &= !0x7; // align to 8 bytes
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), *sp as *mut u8, bytes.len());
            (*sp as *mut u8).add(bytes.len()).write(0); // null terminator
        }
        *sp as u64
    };

    let apple_ptrs: Vec<u64> = apple.iter().map(|s| push_string(&mut sp, s)).collect();
    let envp_ptrs: Vec<u64> = envp.iter().map(|s| push_string(&mut sp, s)).collect();
    let argv_ptrs: Vec<u64> = argv.iter().map(|s| push_string(&mut sp, s)).collect();

    // Align to 16 bytes before the pointer block
    sp &= !0xF;

    // Calculate total entries to ensure 16-byte alignment of final rsp
    let total_entries = 1 + (argv.len() + 1) + (envp.len() + 1) + (apple.len() + 1);
    if total_entries % 2 != 0 {
        sp -= 8; // padding for alignment
    }

    let push_u64 = |sp: &mut usize, val: u64| {
        *sp -= 8;
        unsafe { (*sp as *mut u64).write(val) };
    };

    push_u64(&mut sp, 0);
    for ptr in apple_ptrs.iter().rev() {
        push_u64(&mut sp, *ptr);
    }

    push_u64(&mut sp, 0);
    for ptr in envp_ptrs.iter().rev() {
        push_u64(&mut sp, *ptr);
    }

    push_u64(&mut sp, 0);
    for ptr in argv_ptrs.iter().rev() {
        push_u64(&mut sp, *ptr);
    }

    push_u64(&mut sp, argv.len() as u64);

    sp
}

// ---- Entry ----

pub fn execute(entry_point: u64, argv: &[String], binary_path: &str, is_lc_main: bool, on_stack_ready: impl FnOnce(u64, u64)) -> ! {
    let (stack_base, stack_top) = alloc_stack().expect("failed to allocate stack");
    STACK_BASE_ADDR.store(stack_base as u64, std::sync::atomic::Ordering::Release);
    on_stack_ready(stack_base as u64, STACK_SIZE as u64);
    let sel_ptr = selector_ptr(); // allocates main thread's per-thread selector
    let trampoline = alloc_trampoline_for(sel_ptr).expect("failed to allocate trampoline");
    THREAD_TRAMPOLINE.with(|c| c.set(trampoline));
    let stack_ptr = build_stack(stack_top, argv, binary_path);

    // Try to open /dev/grafted. If it fails, we continue without Mach trap support.
    let fd = unsafe {
        libc::open(
            b"/dev/grafted\0".as_ptr() as *const libc::c_char,
            libc::O_RDWR,
        )
    };
    if fd >= 0 {
        unsafe {
            GRAFTED_FD = fd;
            // Register this process
            let pid = libc::getpid() as u32;
            let _ = grafted_kernel::ioctl::grafted_register(fd, &pid as *const _);
        }
        log::info!("connected to /dev/grafted (fd {})", fd);
    } else {
        log::warn!("failed to open /dev/grafted: Mach traps will not work");
    }

    install_sigsys_handler().expect("failed to install SIGSYS handler");
    enable_sud().expect("failed to enable SUD");

    log::info!("jumping to entry point {entry_point:#x}, argc={}", argv.len());

    // If LC_MAIN, the entry point expects arguments in registers (C calling convention)
    // argc is at [rsp], argv is at [rsp+8], envp is after argv
    let _argc = argv.len() as u64;
    
    // We need to pass argc, argv, envp, apple to the entry point in registers.
    // AND we need to put a return address on the stack that calls exit.
    
    // Set up %gs for Darwin TLS (Go uses %gs:0x30 for the g pointer)
    let gs_page = unsafe {
        libc::mmap(std::ptr::null_mut(), 4096,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS, -1, 0)
    };
    if gs_page != libc::MAP_FAILED {
        unsafe { libc::syscall(libc::SYS_arch_prctl, 0x1001i32 /*ARCH_SET_GS*/, gs_page) };
    }

    let final_stack;
    if is_lc_main {
        // LC_MAIN: entry expects argc in rdi, argv in rsi, return addr on stack.
        let exit_code: [u8; 12] = [0x89, 0xc7, 0xb8, 0xe7, 0x00, 0x00, 0x00, 0x0f, 0x05, 0xcc, 0xcc, 0xcc];
        let exit_addr = unsafe {
            let p = libc::mmap(std::ptr::null_mut(), 4096,
                libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS, -1, 0);
            std::ptr::copy_nonoverlapping(exit_code.as_ptr(), p as *mut u8, exit_code.len());
            p as u64
        };
        final_stack = stack_ptr - 8;
        unsafe { *(final_stack as *mut u64) = exit_addr };

        unsafe {
            asm!(
                "mov byte ptr [r10], {block}",
                "mov rsp, r11",
                "mov rdi, [rsp + 8]",
                "lea rsi, [rsp + 16]",
                "lea rdx, [rsp + 16 + rdi * 8 + 8]",
                "mov rcx, rdx",
                "xor rbp, rbp", "xor rbx, rbx",
                "xor r12, r12", "xor r13, r13", "xor r14, r14", "xor r15, r15",
                "jmp rax",
                in("r10") selector_ptr(), in("r11") final_stack, in("rax") entry_point,
                block = const SYSCALL_DISPATCH_FILTER_BLOCK, options(noreturn),
            );
        }
    } else {
        // LC_UNIXTHREAD: entry reads argc from [rsp], argv from [rsp+8]. No return addr.
        final_stack = stack_ptr;
        unsafe {
            asm!(
                "mov byte ptr [r10], {block}",
                "mov rsp, r11",
                "mov rdi, [rsp]", "lea rsi, [rsp + 8]",
                "xor rbp, rbp", "xor rbx, rbx",
                "xor r12, r12", "xor r13, r13", "xor r14, r14", "xor r15, r15",
                "jmp rax",
                in("r10") selector_ptr(), in("r11") final_stack, in("rax") entry_point,
                block = const SYSCALL_DISPATCH_FILTER_BLOCK, options(noreturn),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_syscall_unix_getpid() {
        // Darwin getpid is 20, Linux is 39.
        let res = unsafe { process_syscall(0x2000014, [0; 6], -1) };
        assert_eq!(res, unsafe { libc::getpid() } as i64);
    }

    #[test]
    fn test_process_syscall_mach_reply_port() {
        // Mach trap 26 (mach_reply_port) → returns fake port 0x307
        let res = unsafe { process_syscall(0x100001a, [0; 6], -1) };
        assert_eq!(res, 0x307);
    }

    #[test]
    fn test_process_syscall_mach_trap_unsupported_fd() {
        // Unimplemented Mach trap 99 with fd=-1 → ENOSYS
        let res = unsafe { process_syscall(0x1000063, [0; 6], -1) };
        assert_eq!(res, -38);
    }
}
