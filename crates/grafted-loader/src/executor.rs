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

use std::sync::atomic::{AtomicPtr, Ordering};

const SYS_USER_DISPATCH: i32 = 2;

// We dynamically allocate the selector to guarantee it's in a writable page.
static SELECTOR_PTR: AtomicPtr<u8> = AtomicPtr::new(std::ptr::null_mut());

pub fn selector_ptr() -> *mut u8 {
    let mut ptr = SELECTOR_PTR.load(Ordering::Acquire);
    if ptr.is_null() {
        let size = NonZeroUsize::new(4096).unwrap();
        ptr = unsafe {
            mmap_anonymous(
                None,
                size,
                ProtFlags::PROT_READ | ProtFlags::PROT_WRITE,
                MapFlags::MAP_PRIVATE,
            )
        }
        .expect("failed to allocate selector page")
        .as_ptr() as *mut u8;
        unsafe { *ptr = SYSCALL_DISPATCH_FILTER_ALLOW };
        
        if SELECTOR_PTR.compare_exchange(
            std::ptr::null_mut(),
            ptr,
            Ordering::Release,
            Ordering::Relaxed,
        ).is_err() {
            // Leak is harmless: one 4KiB page, process-lifetime singleton.
            ptr = SELECTOR_PTR.load(Ordering::Acquire);
        }
    }
    ptr
}

static mut TRAMPOLINE_ADDR: u64 = 0;
static mut GRAFTED_FD: libc::c_int = -1;

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
    let sel_ptr = selector_ptr() as u64;

    //   movabs rax, <selector_ptr>      ; 10 bytes
    //   mov byte ptr [rax], 1           ; 3 bytes
    //   ret                             ; 1 byte
    // Total: 14 bytes
    let mut code = [0u8; 14];
    code[0] = 0x48;
    code[1] = 0xb8;
    code[2..10].copy_from_slice(&sel_ptr.to_le_bytes());
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

// ---- Syscall Processing ----

unsafe fn process_syscall(
    raw_syscall: u64,
    args: [u64; 6],
    fd: libc::c_int,
) -> i64 {
    match syscall::translate(raw_syscall) {
        DarwinSyscall::Unix { linux_nr, .. } if linux_nr >= 0 => {
            // exit/exit_group: terminate immediately
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

            let ret: i64;
            unsafe {
                asm!(
                    "syscall",
                    inlateout("rax") linux_nr as i64 => ret,
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
        }
        DarwinSyscall::Unix { .. } => {
            let msg = b"grafted: unimplemented Darwin syscall\n";
            unsafe { libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len()) };
            -38
        }
        DarwinSyscall::MachTrap { trap_nr } => {
            if fd < 0 {
                let msg = b"grafted: mach trap called but /dev/grafted is not open\n";
                unsafe { libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len()) };
                -38
            } else {
                let mut trap = grafted_kernel::ioctl::GraftedMachTrap {
                    trap_number: trap_nr,
                    args,
                    result: 0,
                };
                let res = unsafe {
                    grafted_kernel::ioctl::grafted_mach_trap(fd, &mut trap)
                };
                if res.is_err() {
                    -38 // Return ENOSYS if ioctl fails
                } else {
                    trap.result
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
    let sel_ptr = selector_ptr();

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
    let sel_ptr = selector_ptr();
    let ret = unsafe {
        libc::prctl(
            PR_SET_SYSCALL_USER_DISPATCH,
            PR_SYS_DISPATCH_ON,
            0_usize,
            0_usize,
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

pub fn execute(entry_point: u64, argv: &[String], binary_path: &str) -> ! {
    let stack_top = alloc_stack().expect("failed to allocate stack");
    let trampoline = alloc_trampoline().expect("failed to allocate trampoline");
    let stack_ptr = build_stack(stack_top, argv, binary_path);

    unsafe { TRAMPOLINE_ADDR = trampoline };

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
    
    // Exit trampoline: xor edi,edi; mov eax,0x2000001 (Darwin exit); syscall
    let exit_trampoline: [u8; 12] = [0x31, 0xff, 0xb8, 0x01, 0x00, 0x00, 0x02, 0x0f, 0x05, 0xcc, 0xcc, 0xcc];
    let exit_trampoline_addr = unsafe {
        let ptr = libc::mmap(
            std::ptr::null_mut(),
            4096,
            libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        );
        std::ptr::copy_nonoverlapping(exit_trampoline.as_ptr(), ptr as *mut u8, exit_trampoline.len());
        ptr as u64
    };

    // Adjust stack pointer to "push" the return address
    let mut final_stack = stack_ptr;
    unsafe {
        final_stack -= 8;
        *(final_stack as *mut u64) = exit_trampoline_addr;
    }
    
    unsafe {
        asm!(
            "mov byte ptr [r10], {block}",
            "mov rsp, r11",
            "mov rdi, [rsp + 8]",       // rdi = argc (now at rsp+8 because of pushed return addr)
            "lea rsi, [rsp + 16]",      // rsi = argv
            "lea rdx, [rsp + 16 + rdi * 8 + 8]", // rdx = envp
            
            "mov rcx, rdx",             // rcx = apple (approximate)

            "xor rbp, rbp",
            "xor rbx, rbx",
            "xor r12, r12",
            "xor r13, r13",
            "xor r14, r14",
            "xor r15, r15",
            "jmp rax",
            in("r10") selector_ptr(),
            in("r11") final_stack,
            in("rax") entry_point,
            block = const SYSCALL_DISPATCH_FILTER_BLOCK,
            options(noreturn),
        );
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
    fn test_process_syscall_mach_trap_unsupported_fd() {
        // Mach trap 26 (mach_reply_port)
        let res = unsafe { process_syscall(0x100001a, [0; 6], -1) };
        assert_eq!(res, -38); // ENOSYS
    }
}
