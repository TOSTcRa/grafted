//! Darwin binary executor.

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
fn alloc_trampoline_for(sel_ptr: *mut u8) -> Result<u64, LoaderError> {
    let sel_addr = sel_ptr as u64;

    //   movabs r11, <sel_ptr>           ; 10 bytes (49 bb ...)
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
                // mmap: translate flags (Darwin MAP_ANON=0x1000 -> Linux=0x20)
                197 | 199 => {
                    let df = a[3] as i32;
                    let mut lf = df & 0x1F;
                    if df & 0x1000 != 0 { lf |= 0x20; }
                    if df & 0x0040 != 0 { lf |= 0x4000; }
                    a[3] = lf as u64;
                    // Guard page (PROT_NONE + MAP_FIXED + MAP_ANON) -> fake success
                    if a[2] == 0 && lf & 0x30 == 0x30 {
                        return if a[0] != 0 { a[0] as i64 } else { 0x7fff_0000 };
                    }
                }
                // mprotect: guard page (PROT_NONE) -> fake success
                74 => {
                    if a[2] == 0 { return 0; }
                }
                // open: translate flags (Darwin O_CREAT=0x200 -> Linux=0x40)
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
                // mach_vm_protect -> mprotect (guard page PROT_NONE -> fake success)
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
                // Intercept tgkill(pid, tid, SIGABRT=6) during lifecycle -
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

    // Leave selector as ALLOW - sigreturn needs it to work.
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
// sigjmp_buf on Linux x86_64 is ~200 bytes and requires 8-byte alignment.
#[repr(C, align(16))]
struct SigJmpBuf([u8; 256]);
static mut RECOVERY_BUF: SigJmpBuf = SigJmpBuf([0u8; 256]);
static RECOVERY_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// During lifecycle: try to skip crashes by walking the frame pointer chain back
static LIFECYCLE_SKIP_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static LIFECYCLE_SKIP_COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Crash sites saved by the signal handler for NOP patching between retries.
#[derive(Clone, Copy)]
struct CrashSite { addr: u64, len: u8 }
static mut CRASH_SITES: [CrashSite; 256] = [CrashSite { addr: 0, len: 0 }; 256];
static CRASH_SITE_COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

#[unsafe(no_mangle)]
pub extern "C" fn grafted_set_lifecycle_skip_mode(active: bool) {
    LIFECYCLE_SKIP_MODE.store(active, std::sync::atomic::Ordering::SeqCst);
    if active {
        LIFECYCLE_SKIP_COUNT.store(0, std::sync::atomic::Ordering::SeqCst);
        CRASH_SITE_COUNT.store(0, std::sync::atomic::Ordering::SeqCst);
    }
}

/// NOP-patch all crash sites saved by the signal handler.
/// Called between retries (outside the signal handler, so mprotect is safe).
#[unsafe(no_mangle)]
pub extern "C" fn grafted_nop_saved_crash_sites() -> u32 {
    let count = CRASH_SITE_COUNT.load(std::sync::atomic::Ordering::Acquire) as usize;
    let mut patched = 0u32;
    for i in 0..count.min(256) {
        let site = unsafe { CRASH_SITES[i] };
        if site.addr == 0 || site.len == 0 { continue; }
        let addr = site.addr as *mut u8;
        let page = (site.addr as usize & !0xFFF) as *mut libc::c_void;
        unsafe {
            if libc::mprotect(page, 8192, libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC) == 0 {
                for k in 0..site.len as usize { *addr.add(k) = 0x90; }
                libc::mprotect(page, 8192, libc::PROT_READ | libc::PROT_EXEC);
                patched += 1;
            }
        }
    }
    // Reset for next retry
    CRASH_SITE_COUNT.store(0, std::sync::atomic::Ordering::Release);
    patched
}

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
        let ret = __sigsetjmp(std::ptr::addr_of_mut!(RECOVERY_BUF.0) as *mut u8, 1);
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

/// Decode a common x86-64 instruction at `rip`. Returns (length, dest_register)
fn decode_binary_insn(rip: u64) -> (u64, i8) {
    let b = |i: u64| -> u8 { unsafe { *((rip + i) as *const u8) } };
    let mut off = 0u64;
    let mut rex: u8 = 0;

    // Skip REX prefix (0x40-0x4F)
    if (b(0) & 0xF0) == 0x40 {
        rex = b(0);
        off = 1;
    }

    let opcode = b(off);

    // Opcodes with ModRM byte - (has_imm_bytes, has_dest_reg)
    let (imm_size, has_dest_reg): (u64, bool) = match opcode {
        0x8B | 0x8A | 0x63 | 0x03 | 0x0B | 0x23 | 0x2B | 0x33 | 0x3B | 0x3A
            => (0, true),  // MOV/MOVSXD/ADD/OR/AND/SUB/XOR/CMP r, r/m
        0x89 | 0x88 | 0x39 | 0x38 | 0x85 | 0x84 | 0x01 | 0x09 | 0x21 | 0x29 | 0x31
            => (0, false), // MOV/CMP/TEST/ADD/OR/AND/SUB/XOR r/m, r (dest is memory/flags)
        0xFF => (0, false), // CALL/JMP/PUSH/INC/DEC r/m
        0x83 | 0x80 | 0xC6 | 0xC7 | 0xF6
            => (1, false), // OP r/m, imm8
        0x81 => (4, false), // OP r/m, imm32
        0x0F => {
            // Two-byte opcode
            let op2 = b(off + 1);
            match op2 {
                0xB6 | 0xB7 | 0xBE | 0xBF | 0xAF
                    => { off += 1; (0, true) }  // MOVZX/MOVSX/IMUL r, r/m
                _ => return (0, -1),
            }
        }
        _ => return (0, -1),
    };

    let modrm = b(off + 1);
    let modf = (modrm >> 6) & 3;
    let reg = ((modrm >> 3) & 7) | if rex & 4 != 0 { 8 } else { 0 };
    let rm = modrm & 7;

    // mod=11: register-to-register (shouldn't fault on memory)
    // Exception: FF /2 = CALL reg, FF /4 = JMP reg
    if modf == 3 {
        if opcode == 0xFF {
            let ext = (modrm >> 3) & 7;
            if ext == 2 || ext == 4 {
                // CALL reg or JMP reg - skip it (3-byte with REX, 2-byte without)
                return (off + 2, -1);
            }
        }
        return (0, -1);
    }

    let has_sib = rm == 4;
    let disp_size: u64 = match modf {
        0 => if rm == 5 { 4 } else { 0 }, // RIP-relative or no disp
        1 => 1,  // disp8
        2 => 4,  // disp32
        _ => 0,
    };

    let insn_len = off + 2 + if has_sib { 1 } else { 0 } + disp_size + imm_size;
    let dest = if has_dest_reg { reg as i8 } else { -1 };
    (insn_len, dest)
}

unsafe extern "C" fn sigsegv_handler(
    _sig: libc::c_int,
    info: *mut libc::siginfo_t,
    context: *mut libc::c_void,
) {
    let info = unsafe { &*info };
    let uctx = unsafe { &*(context as *const libc::ucontext_t) };

    // Re-entrance guard: if the handler itself crashes, longjmp immediately.
    static IN_HANDLER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if IN_HANDLER.swap(true, std::sync::atomic::Ordering::SeqCst) {
        if RECOVERY_ACTIVE.load(std::sync::atomic::Ordering::SeqCst) {
            unsafe { siglongjmp(std::ptr::addr_of_mut!(RECOVERY_BUF.0) as *mut u8, 1) };
        }
        unsafe { libc::_exit(139) };
    }

    if RECOVERY_ACTIVE.load(std::sync::atomic::Ordering::SeqCst) {
        let rip = uctx.uc_mcontext.gregs[libc::REG_RIP as usize] as u64;
        let fault = unsafe { info.si_addr() } as u64;

        if _sig != libc::SIGABRT
            && LIFECYCLE_SKIP_MODE.load(std::sync::atomic::Ordering::Relaxed)
        {
            let skip_n = LIFECYCLE_SKIP_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

            // Handle null function pointer calls: RIP=0 means `call rax` (or
            if skip_n < 500 && rip == 0 {
                let rsp = uctx.uc_mcontext.gregs[libc::REG_RSP as usize] as u64;
                if rsp > 0x1000 && rsp < 0x800000000000 {
                    let ret_addr = unsafe { *(rsp as *const u64) };
                    let uctx_mut = unsafe { &mut *(context as *mut libc::ucontext_t) };
                    uctx_mut.uc_mcontext.gregs[libc::REG_RIP as usize] = ret_addr as i64;
                    uctx_mut.uc_mcontext.gregs[libc::REG_RSP as usize] = (rsp + 8) as i64;
                    uctx_mut.uc_mcontext.gregs[libc::REG_RAX as usize] = 0;
                    uctx_mut.uc_mcontext.gregs[libc::REG_RDX as usize] = 0xE000000000000000u64 as i64;
                    let mut buf = [0u8; 80];
                    let mut p = 0usize;
                    fn w5(b: &mut [u8], p: &mut usize, s: &[u8]) { let n = s.len().min(b.len() - *p); b[*p..*p+n].copy_from_slice(&s[..n]); *p += n; }
                    fn wh5(b: &mut [u8], p: &mut usize, v: u64) { let d = b"0123456789abcdef"; for i in (0..16).rev() { b[*p + 15 - i] = d[((v >> (i*4)) & 0xf) as usize]; } *p += 16; }
                    w5(&mut buf, &mut p, b"  [null-call ret=0x"); wh5(&mut buf, &mut p, ret_addr);
                    w5(&mut buf, &mut p, b"]\n");
                    unsafe { libc::write(2, buf.as_ptr() as *const _, p) };
                    IN_HANDLER.store(false, std::sync::atomic::Ordering::SeqCst);
                    return;
                }
            }

            // For crashes IN THE BINARY: decode the instruction, advance RIP
            if skip_n < 500 && rip >= 0x100000000 && rip < 0x100200000 {
                let (insn_len, dest_reg) = decode_binary_insn(rip);
                if insn_len > 0 {
                    // DIAGNOSTIC DUMP: Show instruction bytes and all registers to identify failing register
                    if rip == 0x10006fcd1 {
                        let mut buf = [0u8; 512];
                        let mut p = 0usize;
                        fn wdiag(b: &mut [u8], p: &mut usize, s: &[u8]) {
                            let n = s.len().min(b.len() - *p);
                            b[*p..*p+n].copy_from_slice(&s[..n]);
                            *p += n;
                        }
                        fn whdiag(b: &mut [u8], p: &mut usize, v: u64) {
                            let d = b"0123456789abcdef";
                            for i in (0..16).rev() {
                                b[*p + 15 - i] = d[((v >> (i*4)) & 0xf) as usize];
                            }
                            *p += 16;
                        }

                        wdiag(&mut buf, &mut p, b"\n=== DIAGNOSTIC DUMP at 0x10006fcd1 ===\n");

                        // Dump instruction bytes
                        wdiag(&mut buf, &mut p, b"INSN: ");
                        let insn_bytes = unsafe { std::slice::from_raw_parts(rip as *const u8, insn_len as usize) };
                        for &byte in insn_bytes {
                            let d = b"0123456789abcdef";
                            wdiag(&mut buf, &mut p, &[d[(byte >> 4) as usize]]);
                            wdiag(&mut buf, &mut p, &[d[(byte & 0xf) as usize]]);
                            wdiag(&mut buf, &mut p, b" ");
                        }
                        wdiag(&mut buf, &mut p, b"\n");

                        // Dump all CPU registers
                        wdiag(&mut buf, &mut p, b"RAX=0x"); whdiag(&mut buf, &mut p, uctx.uc_mcontext.gregs[libc::REG_RAX as usize] as u64);
                        wdiag(&mut buf, &mut p, b" RBX=0x"); whdiag(&mut buf, &mut p, uctx.uc_mcontext.gregs[libc::REG_RBX as usize] as u64);
                        wdiag(&mut buf, &mut p, b"\nRCX=0x"); whdiag(&mut buf, &mut p, uctx.uc_mcontext.gregs[libc::REG_RCX as usize] as u64);
                        wdiag(&mut buf, &mut p, b" RDX=0x"); whdiag(&mut buf, &mut p, uctx.uc_mcontext.gregs[libc::REG_RDX as usize] as u64);
                        wdiag(&mut buf, &mut p, b"\nRSI=0x"); whdiag(&mut buf, &mut p, uctx.uc_mcontext.gregs[libc::REG_RSI as usize] as u64);
                        wdiag(&mut buf, &mut p, b" RDI=0x"); whdiag(&mut buf, &mut p, uctx.uc_mcontext.gregs[libc::REG_RDI as usize] as u64);
                        wdiag(&mut buf, &mut p, b"\nRSP=0x"); whdiag(&mut buf, &mut p, uctx.uc_mcontext.gregs[libc::REG_RSP as usize] as u64);
                        wdiag(&mut buf, &mut p, b" RBP=0x"); whdiag(&mut buf, &mut p, uctx.uc_mcontext.gregs[libc::REG_RBP as usize] as u64);
                        wdiag(&mut buf, &mut p, b"\nR8 =0x"); whdiag(&mut buf, &mut p, uctx.uc_mcontext.gregs[libc::REG_R8 as usize] as u64);
                        wdiag(&mut buf, &mut p, b" R9 =0x"); whdiag(&mut buf, &mut p, uctx.uc_mcontext.gregs[libc::REG_R9 as usize] as u64);
                        wdiag(&mut buf, &mut p, b"\nR10=0x"); whdiag(&mut buf, &mut p, uctx.uc_mcontext.gregs[libc::REG_R10 as usize] as u64);
                        wdiag(&mut buf, &mut p, b" R11=0x"); whdiag(&mut buf, &mut p, uctx.uc_mcontext.gregs[libc::REG_R11 as usize] as u64);
                        wdiag(&mut buf, &mut p, b"\nR12=0x"); whdiag(&mut buf, &mut p, uctx.uc_mcontext.gregs[libc::REG_R12 as usize] as u64);
                        wdiag(&mut buf, &mut p, b" R13=0x"); whdiag(&mut buf, &mut p, uctx.uc_mcontext.gregs[libc::REG_R13 as usize] as u64);
                        wdiag(&mut buf, &mut p, b"\nR14=0x"); whdiag(&mut buf, &mut p, uctx.uc_mcontext.gregs[libc::REG_R14 as usize] as u64);
                        wdiag(&mut buf, &mut p, b" R15=0x"); whdiag(&mut buf, &mut p, uctx.uc_mcontext.gregs[libc::REG_R15 as usize] as u64);
                        wdiag(&mut buf, &mut p, b"\nFAULT=0x"); whdiag(&mut buf, &mut p, fault);
                        wdiag(&mut buf, &mut p, b" dest_reg=");
                        if dest_reg >= 0 {
                            wdiag(&mut buf, &mut p, &[b'0' + dest_reg as u8]);
                        } else {
                            wdiag(&mut buf, &mut p, b"-1");
                        }
                        wdiag(&mut buf, &mut p, b"\n=== END DIAGNOSTIC ===\n");
                        unsafe { libc::write(2, buf.as_ptr() as *const _, p) };
                    }

                    // Save crash site for NOP patching between retries
                    let idx = CRASH_SITE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) as usize;
                    if idx < 256 {
                        unsafe { CRASH_SITES[idx] = CrashSite { addr: rip, len: insn_len as u8 }; }
                    }

                    let uctx_mut = unsafe { &mut *(context as *mut libc::ucontext_t) };
                    uctx_mut.uc_mcontext.gregs[libc::REG_RIP as usize] = (rip + insn_len) as i64;

                    // Zero destination register if known
                    if dest_reg >= 0 {
                        let greg_idx: i32 = match dest_reg {
                            0 => libc::REG_RAX, 1 => libc::REG_RCX, 2 => libc::REG_RDX,
                            3 => libc::REG_RBX, 4 => libc::REG_RSP, 5 => libc::REG_RBP,
                            6 => libc::REG_RSI, 7 => libc::REG_RDI, 8 => libc::REG_R8,
                            9 => libc::REG_R9, 10 => libc::REG_R10, 11 => libc::REG_R11,
                            12 => libc::REG_R12, 13 => libc::REG_R13, 14 => libc::REG_R14,
                            15 => libc::REG_R15, _ => -1,
                        };
                        if greg_idx >= 0 {
                            uctx_mut.uc_mcontext.gregs[greg_idx as usize] = 0;
                        }
                    }

                    // Signal-safe logging
                    let mut buf = [0u8; 128];
                    let mut p = 0usize;
                    fn w3(b: &mut [u8], p: &mut usize, s: &[u8]) { let n = s.len().min(b.len() - *p); b[*p..*p+n].copy_from_slice(&s[..n]); *p += n; }
                    fn wh3(b: &mut [u8], p: &mut usize, v: u64) { let d = b"0123456789abcdef"; for i in (0..16).rev() { b[*p + 15 - i] = d[((v >> (i*4)) & 0xf) as usize]; } *p += 16; }
                    w3(&mut buf, &mut p, b"  [skip#");
                    let mut num = skip_n; let mut digits = [0u8; 10]; let mut nd = 0;
                    loop { digits[nd] = b'0' + (num % 10) as u8; nd += 1; num /= 10; if num == 0 { break; } }
                    for i in (0..nd).rev() { w3(&mut buf, &mut p, &digits[i..i+1]); }
                    w3(&mut buf, &mut p, b"] rip=0x"); wh3(&mut buf, &mut p, rip);
                    w3(&mut buf, &mut p, b" +"); w3(&mut buf, &mut p, &[b'0' + insn_len as u8]);
                    w3(&mut buf, &mut p, b"\n");
                    unsafe { libc::write(2, buf.as_ptr() as *const _, p) };
                    IN_HANDLER.store(false, std::sync::atomic::Ordering::SeqCst);
                    return;
                }
            }

            // For crashes in libraries or non-decodable instructions: longjmp + retry.
            // swift_once predicates prevent re-executing already-ran callbacks.
            {
                let mut buf = [0u8; 128];
                let mut p = 0usize;
                fn w4(b: &mut [u8], p: &mut usize, s: &[u8]) { let n = s.len().min(b.len() - *p); b[*p..*p+n].copy_from_slice(&s[..n]); *p += n; }
                fn wh4(b: &mut [u8], p: &mut usize, v: u64) { let d = b"0123456789abcdef"; for i in (0..16).rev() { b[*p + 15 - i] = d[((v >> (i*4)) & 0xf) as usize]; } *p += 16; }
                w4(&mut buf, &mut p, b"  [crash#");
                let mut num = skip_n; let mut digits = [0u8; 10]; let mut nd = 0;
                loop { digits[nd] = b'0' + (num % 10) as u8; nd += 1; num /= 10; if num == 0 { break; } }
                for i in (0..nd).rev() { w4(&mut buf, &mut p, &digits[i..i+1]); }
                w4(&mut buf, &mut p, b"] f=0x"); wh4(&mut buf, &mut p, fault);
                w4(&mut buf, &mut p, b" rip=0x"); wh4(&mut buf, &mut p, rip);
                w4(&mut buf, &mut p, b"\n");
                unsafe { libc::write(2, buf.as_ptr() as *const _, p) };
            }
        }

        // Longjmp to recovery point
        IN_HANDLER.store(false, std::sync::atomic::Ordering::SeqCst);
        unsafe { siglongjmp(std::ptr::addr_of_mut!(RECOVERY_BUF.0) as *mut u8, 1) };
    }
    IN_HANDLER.store(false, std::sync::atomic::Ordering::SeqCst);
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
            let val = unsafe { std::ptr::read_unaligned((rdi + i * 8) as *const u64) };
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
        install_sigsegv();
    }
    Ok(())
}

fn install_sigsegv() {
    unsafe {
        // Set up alternate signal stack so handler works even on stack overflow
        static mut ALT_STACK: [u8; 65536] = [0u8; 65536];
        let ss = libc::stack_t {
            ss_sp: std::ptr::addr_of_mut!(ALT_STACK) as *mut _,
            ss_flags: 0,
            ss_size: 65536,
        };
        libc::sigaltstack(&ss, std::ptr::null_mut());

        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = sigsegv_handler as *const () as usize;
        sa.sa_flags = libc::SA_SIGINFO | libc::SA_NODEFER | libc::SA_ONSTACK;
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(libc::SIGSEGV, &sa, std::ptr::null_mut());
        libc::sigaction(libc::SIGBUS, &sa, std::ptr::null_mut());
        libc::sigaction(libc::SIGABRT, &sa, std::ptr::null_mut());
    }
}

/// Override libc abort() - during lifecycle recovery, longjmp instead of aborting.
#[unsafe(no_mangle)]
pub extern "C" fn abort() -> ! {
    if RECOVERY_ACTIVE.load(std::sync::atomic::Ordering::SeqCst) {
        // Print caller return address for diagnostics
        let ra = unsafe {
            let r: u64;
            std::arch::asm!("mov 8(%rbp), {0}", out(reg) r, options(nostack, att_syntax));
            r
        };
        let mut dli: libc::Dl_info = unsafe { std::mem::zeroed() };
        let has_dl = unsafe { libc::dladdr(ra as *const _, &mut dli) } != 0;
        if has_dl && !dli.dli_fname.is_null() {
            let lib = unsafe { std::ffi::CStr::from_ptr(dli.dli_fname) }.to_string_lossy();
            let off = ra - dli.dli_fbase as u64;
            log::warn!("abort() intercepted - caller: {:#x} (off={:#x} {})", ra, off, lib);
        } else {
            log::warn!("abort() intercepted - caller: {:#x}", ra);
        }
        unsafe { siglongjmp(std::ptr::addr_of_mut!(RECOVERY_BUF.0) as *mut u8, 1) };
    }
    // Real abort for non-lifecycle crashes
    unsafe { libc::_exit(134) }; // 128 + SIGABRT(6)
}

/// Re-install our SIGSEGV handler (Swift runtime may override it).
#[unsafe(no_mangle)]
pub extern "C" fn grafted_reinstall_sigsegv_handler() {
    install_sigsegv();
    log::info!("Re-installed SIGSEGV handler after Swift runtime init");
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
    log::info!("Go runtime.settls patched - arch_prctl(ARCH_SET_GS)");
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
        // Mach trap 26 (mach_reply_port) -> returns fake port 0x307
        let res = unsafe { process_syscall(0x100001a, [0; 6], -1) };
        assert_eq!(res, 0x307);
    }

    #[test]
    fn test_process_syscall_mach_trap_unsupported_fd() {
        // Unimplemented Mach trap 99 with fd=-1 -> ENOSYS
        let res = unsafe { process_syscall(0x1000063, [0; 6], -1) };
        assert_eq!(res, -38);
    }
}
