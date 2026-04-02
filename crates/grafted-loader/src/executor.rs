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
use std::num::NonZeroUsize;

use nix::sys::mman::{mmap_anonymous, MapFlags, ProtFlags};

use grafted_kernel::syscall::{self, DarwinSyscall};

use crate::error::LoaderError;

// ---- SUD constants ----

const PR_SET_SYSCALL_USER_DISPATCH: libc::c_int = 59;
const PR_SYS_DISPATCH_ON: libc::c_int = 1;
const SYSCALL_DISPATCH_FILTER_ALLOW: u8 = 0;
const SYSCALL_DISPATCH_FILTER_BLOCK: u8 = 1;

const SYS_USER_DISPATCH: i32 = 2;

static mut SELECTOR: u8 = SYSCALL_DISPATCH_FILTER_ALLOW;
static mut TRAMPOLINE_ADDR: u64 = 0;

pub fn selector_ptr() -> *mut u8 {
    (&raw mut SELECTOR) as *mut u8
}

// ---- Stack allocation ----

const STACK_SIZE: usize = 8 * 1024 * 1024;

fn alloc_stack() -> Result<usize, LoaderError> {
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

    let top = base.as_ptr() as usize + STACK_SIZE;
    Ok(top & !0xF)
}

// ---- Trampoline ----

/// Allocate an executable page containing a trampoline that:
///   1. Sets SELECTOR = BLOCK
///   2. Jumps to the address stored in r15 (which we set in the signal handler)
///
/// We store the real return RIP in r15 (callee-saved, Darwin code shouldn't use it
/// between syscalls since it's our register now).
fn alloc_trampoline() -> Result<u64, LoaderError> {
    let selector_ptr = (&raw mut SELECTOR) as u64;

    // Machine code for the trampoline:
    //   movabs rax, <selector_ptr>      ; 10 bytes
    //   mov byte ptr [rax], 1           ; 3 bytes  (BLOCK = 1)
    //   jmp r15                         ; 3 bytes
    // Total: 16 bytes
    let mut code = [0u8; 16];
    // movabs rax, imm64
    code[0] = 0x48;
    code[1] = 0xb8;
    code[2..10].copy_from_slice(&selector_ptr.to_le_bytes());
    // mov byte ptr [rax], 1
    code[10] = 0xc6;
    code[11] = 0x00;
    code[12] = SYSCALL_DISPATCH_FILTER_BLOCK;
    // jmp r15
    code[13] = 0x41;
    code[14] = 0xff;
    code[15] = 0xe7;

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
    log::debug!("trampoline at {addr:#x}");
    Ok(addr)
}

// ---- SIGSYS signal handler ----

unsafe extern "C" fn sigsys_handler(
    _sig: libc::c_int,
    info: *mut libc::siginfo_t,
    context: *mut libc::c_void,
) {
    // FIRST: allow our own syscalls
    unsafe { SELECTOR = SYSCALL_DISPATCH_FILTER_ALLOW };

    let info = unsafe { &*info };
    if info.si_code != SYS_USER_DISPATCH {
        unsafe { SELECTOR = SYSCALL_DISPATCH_FILTER_BLOCK };
        return;
    }

    let uctx = unsafe { &mut *(context as *mut libc::ucontext_t) };
    let gregs = &mut uctx.uc_mcontext.gregs;

    let raw_syscall = gregs[libc::REG_RAX as usize] as u64;
    let arg1 = gregs[libc::REG_RDI as usize] as u64;
    let arg2 = gregs[libc::REG_RSI as usize] as u64;
    let arg3 = gregs[libc::REG_RDX as usize] as u64;
    let arg4 = gregs[libc::REG_R10 as usize] as u64;
    let arg5 = gregs[libc::REG_R8 as usize] as u64;
    let arg6 = gregs[libc::REG_R9 as usize] as u64;

    let result: i64 = match syscall::translate(raw_syscall) {
        DarwinSyscall::Unix { linux_nr, .. } if linux_nr >= 0 => {
            // exit/exit_group: terminate immediately
            if linux_nr == 60 || linux_nr == 231 {
                unsafe {
                    asm!(
                        "syscall",
                        in("rax") 231_i64,
                        in("rdi") arg1,
                        options(noreturn, nostack),
                    );
                }
            }

            let ret: i64;
            unsafe {
                asm!(
                    "syscall",
                    inlateout("rax") linux_nr as i64 => ret,
                    in("rdi") arg1,
                    in("rsi") arg2,
                    in("rdx") arg3,
                    in("r10") arg4,
                    in("r8") arg5,
                    in("r9") arg6,
                    lateout("rcx") _,
                    lateout("r11") _,
                    options(nostack),
                );
            }
            ret
        }
        DarwinSyscall::Unix { .. } => {
            let msg = b"grafted: unimplemented Darwin syscall\n";
            unsafe { libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len()) };
            -38
        }
        DarwinSyscall::MachTrap { .. } => {
            let msg = b"grafted: unimplemented Mach trap\n";
            unsafe { libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len()) };
            -38
        }
        DarwinSyscall::Unknown { .. } => -38,
    };

    gregs[libc::REG_RAX as usize] = result;

    // Redirect return to the trampoline:
    // Save the real resume RIP in r15, set RIP to trampoline.
    // The trampoline will set SELECTOR=BLOCK then jmp r15.
    // This way sigreturn runs with ALLOW (works), and the trampoline
    // re-arms interception before Darwin code resumes.
    let real_rip = gregs[libc::REG_RIP as usize];
    gregs[libc::REG_R15 as usize] = real_rip;
    gregs[libc::REG_RIP as usize] = unsafe { TRAMPOLINE_ADDR } as i64;

    // Leave selector as ALLOW — sigreturn needs it to work.
    // The trampoline will set it back to BLOCK.
}

// ---- SUD setup ----

fn enable_sud() -> Result<(), LoaderError> {
    let selector_ptr = (&raw mut SELECTOR) as *mut u8;
    let ret = unsafe {
        libc::prctl(
            PR_SET_SYSCALL_USER_DISPATCH,
            PR_SYS_DISPATCH_ON,
            0_usize,
            0_usize,
            selector_ptr as usize,
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
    }
    Ok(())
}

// ---- Entry ----

pub fn execute(entry_point: u64) -> ! {
    let stack_top = alloc_stack().expect("failed to allocate stack");
    let trampoline = alloc_trampoline().expect("failed to allocate trampoline");

    unsafe { TRAMPOLINE_ADDR = trampoline };

    install_sigsys_handler().expect("failed to install SIGSYS handler");
    enable_sud().expect("failed to enable SUD");

    log::info!("jumping to entry point {entry_point:#x}");

    let selector_ptr = (&raw mut SELECTOR) as *mut u8;

    unsafe {
        asm!(
            "mov byte ptr [{selector}], {block}",
            "mov rsp, {stack}",
            "push 0",
            "xor rbp, rbp",
            "xor rbx, rbx",
            "xor r12, r12",
            "xor r13, r13",
            "xor r14, r14",
            "xor r15, r15",
            "jmp {entry}",
            selector = in(reg) selector_ptr,
            block = const SYSCALL_DISPATCH_FILTER_BLOCK,
            stack = in(reg) stack_top,
            entry = in(reg) entry_point,
            options(noreturn),
        );
    }
}
