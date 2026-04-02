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
///   2. Returns to the real RIP (pushed on stack by the signal handler)
///
/// The signal handler pushes the real resume RIP onto the Darwin stack
/// and sets RIP to this trampoline. The trampoline re-arms SUD then `ret`s back.
/// This avoids clobbering any callee-saved registers.
fn alloc_trampoline() -> Result<u64, LoaderError> {
    let selector_ptr = (&raw mut SELECTOR) as u64;

    //   movabs rax, <selector_ptr>      ; 10 bytes
    //   mov byte ptr [rax], 1           ; 3 bytes
    //   ret                             ; 1 byte
    // Total: 14 bytes
    let mut code = [0u8; 14];
    code[0] = 0x48;
    code[1] = 0xb8;
    code[2..10].copy_from_slice(&selector_ptr.to_le_bytes());
    code[10] = 0xc6;
    code[11] = 0x00;
    code[12] = SYSCALL_DISPATCH_FILTER_BLOCK;
    code[13] = 0xc3; // ret

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

    // Push the real resume RIP onto the Darwin stack, then redirect to trampoline.
    // The trampoline sets SELECTOR=BLOCK then `ret` pops the real RIP.
    // This avoids clobbering any registers the Darwin code uses.
    let real_rip = gregs[libc::REG_RIP as usize] as u64;
    let rsp = gregs[libc::REG_RSP as usize] as u64;
    let new_rsp = rsp - 8;
    unsafe { (new_rsp as *mut u64).write(real_rip) };
    gregs[libc::REG_RSP as usize] = new_rsp as i64;
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

// ---- Stack layout for LC_MAIN ----

/// Build a Darwin-compatible process stack with argc/argv/envp/apple[].
/// Returns the address of argc (where rsp should point on entry).
///
/// Layout (high to low):
///   string data (argv[i], envp[i], apple[i] C strings)
///   NULL
///   apple[0..n] pointers
///   NULL
///   envp[0..n] pointers
///   NULL
///   argv[0..n] pointers
///   argc (u64)          ← returned address
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

    // Helper: push a C string onto the stack, return its address
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

    // Push all strings first (they go at the top of the stack)
    let apple_ptrs: Vec<u64> = apple.iter().map(|s| push_string(&mut sp, s)).collect();
    let envp_ptrs: Vec<u64> = envp.iter().map(|s| push_string(&mut sp, s)).collect();
    let argv_ptrs: Vec<u64> = argv.iter().map(|s| push_string(&mut sp, s)).collect();

    // Now push the pointer arrays and argc (growing downward)
    // Align to 16 bytes before the pointer block
    sp &= !0xF;

    // Calculate total entries to ensure 16-byte alignment of final rsp
    // argc(1) + argv(n+1) + envp(n+1) + apple(n+1) = entries
    let total_entries = 1 + (argv.len() + 1) + (envp.len() + 1) + (apple.len() + 1);
    // If total_entries is odd, the stack won't be 16-byte aligned; add padding
    if total_entries % 2 != 0 {
        sp -= 8; // padding for alignment
    }

    let push_u64 = |sp: &mut usize, val: u64| {
        *sp -= 8;
        unsafe { (*sp as *mut u64).write(val) };
    };

    // Push apple[] (NULL terminated, reverse order)
    push_u64(&mut sp, 0); // NULL terminator
    for ptr in apple_ptrs.iter().rev() {
        push_u64(&mut sp, *ptr);
    }

    // Push envp[] (NULL terminated, reverse order)
    push_u64(&mut sp, 0);
    for ptr in envp_ptrs.iter().rev() {
        push_u64(&mut sp, *ptr);
    }

    // Push argv[] (NULL terminated, reverse order)
    push_u64(&mut sp, 0);
    for ptr in argv_ptrs.iter().rev() {
        push_u64(&mut sp, *ptr);
    }

    // Push argc
    push_u64(&mut sp, argv.len() as u64);

    sp
}

// ---- Entry ----

pub fn execute(entry_point: u64, argv: &[String], binary_path: &str) -> ! {
    let stack_top = alloc_stack().expect("failed to allocate stack");
    let trampoline = alloc_trampoline().expect("failed to allocate trampoline");
    let stack_ptr = build_stack(stack_top, argv, binary_path);

    unsafe { TRAMPOLINE_ADDR = trampoline };

    install_sigsys_handler().expect("failed to install SIGSYS handler");
    enable_sud().expect("failed to enable SUD");

    log::info!("jumping to entry point {entry_point:#x}, argc={}", argv.len());

    let selector_ptr = (&raw mut SELECTOR) as *mut u8;

    unsafe {
        asm!(
            "mov byte ptr [{selector}], {block}",
            "mov rsp, {stack}",
            "xor rbp, rbp",
            "xor rbx, rbx",
            "xor r12, r12",
            "xor r13, r13",
            "xor r14, r14",
            "xor r15, r15",
            "jmp {entry}",
            selector = in(reg) selector_ptr,
            block = const SYSCALL_DISPATCH_FILTER_BLOCK,
            stack = in(reg) stack_ptr,
            entry = in(reg) entry_point,
            options(noreturn),
        );
    }
}
