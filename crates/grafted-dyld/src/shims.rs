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

// These hold pointers to the real Linux FILE structs.
// Darwin code does: FILE *out = *___stdoutp; fprintf(out, ...).
// By pointing these to real Linux FILE*, all stdio works natively.
static mut REAL_STDIN: *mut libc::FILE = std::ptr::null_mut();
static mut REAL_STDOUT: *mut libc::FILE = std::ptr::null_mut();
static mut REAL_STDERR: *mut libc::FILE = std::ptr::null_mut();
// __runetype is at byte offset 0x3c (60) from _RuneLocale start.
// 256 entries of u32. Total buffer: 60 + 256*4 = 1084 bytes. Round up.
#[repr(C, align(8))]
struct RuneLocale([u8; 2048]);
static mut DEFAULT_RUNE_LOCALE: RuneLocale = RuneLocale([0; 2048]);

fn init_rune_locale() {
    const CT_C: u32 = 0x200;
    const CT_D: u32 = 0x400;
    const CT_L: u32 = 0x2000;
    const CT_S: u32 = 0x4000;
    const CT_U: u32 = 0x8000;
    const CT_A: u32 = 0x100;
    const CT_P: u32 = 0x10000;
    const CT_X: u32 = 0x100000;
    const CT_R: u32 = 0x40000;
    const CT_B: u32 = 0x20000;

    let buf = unsafe { &mut (*(&raw mut DEFAULT_RUNE_LOCALE)).0 };
    // Write __runetype[c] at byte offset 0x3c + c*4
    for c in 0u32..128 {
        let mut r = 0u32;
        let ch = c as u8;
        if ch < 0x20 || ch == 0x7f { r |= CT_C; }
        if ch >= b'0' && ch <= b'9' { r |= CT_D | CT_X | CT_R; }
        if ch >= b'a' && ch <= b'z' { r |= CT_L | CT_A | CT_R; }
        if ch >= b'A' && ch <= b'Z' { r |= CT_U | CT_A | CT_R; }
        if ch >= b'a' && ch <= b'f' { r |= CT_X; }
        if ch >= b'A' && ch <= b'F' { r |= CT_X; }
        if ch == b' ' || ch == b'\t' { r |= CT_S | CT_B; }
        if ch == b'\n' || ch == b'\r' || ch == 0x0b || ch == 0x0c { r |= CT_S; }
        if ch >= 0x20 && ch <= 0x7e { r |= CT_R; }
        if ch >= 0x21 && ch <= 0x7e && r & (CT_A | CT_D) == 0 { r |= CT_P; }

        let off = 0x3c + (c as usize) * 4;
        buf[off..off+4].copy_from_slice(&r.to_le_bytes());
    }
}

unsafe extern "C" {
    pub fn bzero(s: *mut libc::c_void, n: libc::size_t);
    pub fn clock() -> libc::clock_t;
    pub fn acos(x: f64) -> f64;
    pub fn asin(x: f64) -> f64;
    pub fn atan(x: f64) -> f64;
    pub fn atan2(y: f64, x: f64) -> f64;
    pub fn cos(x: f64) -> f64;
    pub fn sin(x: f64) -> f64;
    pub fn tan(x: f64) -> f64;
    pub fn exp(x: f64) -> f64;
    #[link_name = "log"]
    pub fn c_log(x: f64) -> f64;
    pub fn log10(x: f64) -> f64;
    pub fn log2(x: f64) -> f64;
    pub fn pow(x: f64, y: f64) -> f64;
    pub fn fmod(x: f64, y: f64) -> f64;
    pub fn frexp(x: f64, exp: *mut libc::c_int) -> f64;
    pub fn ldexp(x: f64, exp: libc::c_int) -> f64;
    pub fn flockfile(file: *mut libc::FILE);
    pub fn funlockfile(file: *mut libc::FILE);
    #[link_name = "_setjmp"]
    pub fn _setjmp(env: *mut libc::c_void) -> libc::c_int;
    #[link_name = "_longjmp"]
    pub fn _longjmp(env: *mut libc::c_void, val: libc::c_int) -> !;

    // Math functions needed by jq and other real binaries
    pub fn acosh(x: f64) -> f64;
    pub fn asinh(x: f64) -> f64;
    pub fn atanh(x: f64) -> f64;
    pub fn cbrt(x: f64) -> f64;
    pub fn cosh(x: f64) -> f64;
    pub fn sinh(x: f64) -> f64;
    pub fn tanh(x: f64) -> f64;
    pub fn erf(x: f64) -> f64;
    pub fn erfc(x: f64) -> f64;
    pub fn expm1(x: f64) -> f64;
    pub fn fdim(x: f64, y: f64) -> f64;
    pub fn fma(x: f64, y: f64, z: f64) -> f64;
    pub fn hypot(x: f64, y: f64) -> f64;
    pub fn j0(x: f64) -> f64;
    pub fn j1(x: f64) -> f64;
    pub fn jn(n: libc::c_int, x: f64) -> f64;
    pub fn lgamma(x: f64) -> f64;
    pub fn lgamma_r(x: f64, signgamp: *mut libc::c_int) -> f64;
    pub fn log1p(x: f64) -> f64;
    pub fn logb(x: f64) -> f64;
    pub fn modf(x: f64, iptr: *mut f64) -> f64;
    pub fn nextafter(x: f64, y: f64) -> f64;
    pub fn nexttoward(x: f64, y: f64) -> f64;
    pub fn remainder(x: f64, y: f64) -> f64;
    pub fn scalb(x: f64, y: f64) -> f64;
    pub fn scalbln(x: f64, n: libc::c_long) -> f64;
    pub fn tgamma(x: f64) -> f64;
    pub fn y0(x: f64) -> f64;
    pub fn y1(x: f64) -> f64;
    pub fn yn(n: libc::c_int, x: f64) -> f64;
    pub fn exp2(x: f64) -> f64;
    // __exp10 is Darwin-only; we implement it as pow(10, x)

    // String/misc
    pub fn memmem(haystack: *const libc::c_void, hlen: libc::size_t, needle: *const libc::c_void, nlen: libc::size_t) -> *mut libc::c_void;
    pub fn vsnprintf(s: *mut i8, n: libc::size_t, fmt: *const i8, ap: *mut libc::c_void) -> libc::c_int;

    // glibc fortified functions (same signature as Darwin's _chk variants)
    pub fn __snprintf_chk(s: *mut i8, maxlen: libc::size_t, flag: libc::c_int, slen: libc::size_t, fmt: *const i8, ...) -> libc::c_int;
    pub fn __sprintf_chk(s: *mut i8, flag: libc::c_int, slen: libc::size_t, fmt: *const i8, ...) -> libc::c_int;
    pub fn timegm(tm: *mut libc::tm) -> libc::time_t;

    // Float variants
    pub fn sqrtf(x: f32) -> f32;
    pub fn sqrt(x: f64) -> f64;
    pub fn floorf(x: f32) -> f32;
    pub fn floor(x: f64) -> f64;
    pub fn ceilf(x: f32) -> f32;
    pub fn ceil(x: f64) -> f64;
    pub fn cosf(x: f32) -> f32;
    pub fn sinf(x: f32) -> f32;
    pub fn tanf(x: f32) -> f32;
    pub fn expf(x: f32) -> f32;
    pub fn logf(x: f32) -> f32;
    pub fn fmodf(x: f32, y: f32) -> f32;
    pub fn fmaxf(x: f32, y: f32) -> f32;
    pub fn fminf(x: f32, y: f32) -> f32;
    pub fn fmaxl(x: f64, y: f64) -> f64;
    pub fn fabsf(x: f32) -> f32;
    pub fn fabs(x: f64) -> f64;
    pub fn roundf(x: f32) -> f32;
    pub fn round(x: f64) -> f64;
    pub fn truncf(x: f32) -> f32;
    pub fn trunc(x: f64) -> f64;
}

unsafe extern "C" {
    // Linux libc exposes stdin/stdout/stderr as global *mut FILE pointers
    static stdin: *mut libc::FILE;
    static stdout: *mut libc::FILE;
    static stderr: *mut libc::FILE;
}

unsafe fn setup_darwin_stdio() {
    unsafe {
        REAL_STDIN = stdin;
        REAL_STDOUT = stdout;
        REAL_STDERR = stderr;
    }
}

/// Generate an executable trampoline that wraps a function pointer:
///   mov byte [selector_ptr], ALLOW
///   movabs rax, <target_fn>
///   call rax
///   mov byte [selector_ptr], BLOCK
///   ret
/// Returns the trampoline's address. All trampolines are allocated from
/// a single mmap'd executable page pool.
fn gen_trampoline(target: u64) -> u64 {
    use std::sync::Mutex;
    // Wrap raw pointer in a Send-able newtype for the static Mutex
    struct PoolPtr(*mut u8);
    unsafe impl Send for PoolPtr {}
    static POOL: Mutex<Option<(PoolPtr, usize)>> = Mutex::new(None);
    const PAGE_SIZE: usize = 4096 * 16;
    const TRAMP_SIZE: usize = 120; // pre + post + 8-byte saved_ret slot at end

    // Use the address of SELECTOR_PTR itself (an AtomicPtr), not its current value.
    // The trampoline will load the actual selector address at runtime via double indirection.
    let selector_ptr_addr = (&raw const SELECTOR_PTR) as u64;

    let mut pool = POOL.lock().unwrap();
    let (base, offset) = pool.get_or_insert_with(|| {
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                PAGE_SIZE,
                libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1, 0,
            )
        };
        assert_ne!(ptr, libc::MAP_FAILED);
        (PoolPtr(ptr as *mut u8), 0)
    });

    assert!(*offset + TRAMP_SIZE <= PAGE_SIZE, "trampoline pool exhausted");
    let tramp = unsafe { base.0.add(*offset) };
    *offset += TRAMP_SIZE;

    // Trampoline with return-address swap.
    // Each trampoline has its own 8-byte slot for the saved return address
    // at the END of the trampoline (byte 88..95). This avoids the nested-call
    // problem of a single global.

    let spa = selector_ptr_addr.to_le_bytes();
    let tgt = target.to_le_bytes();
    // saved_ret is embedded at the end of this trampoline's slot (offset 112)
    let saved_ret_addr = (tramp as u64) + 112;
    let sra = saved_ret_addr.to_le_bytes();

    // Layout: PRE (set ALLOW, swap ret, jmp) + POST (set BLOCK, jmp to saved ret)
    // POST is at a fixed offset within this trampoline slot.
    let mut code = [0u8; TRAMP_SIZE];
    let mut p = 0;

    // --- PRE ---
    // Save [rsp] (original ret addr) into SAVED_RET global
    code[p]=0x4c; code[p+1]=0x8b; code[p+2]=0x1c; code[p+3]=0x24; p+=4; // mov r11, [rsp]
    code[p]=0x49; code[p+1]=0xba; p+=2;                                    // movabs r10, &SAVED_RET
    code[p..p+8].copy_from_slice(&sra); p+=8;
    code[p]=0x4d; code[p+1]=0x89; code[p+2]=0x1a; p+=3;                    // mov [r10], r11

    // Replace [rsp] with address of POST stub (rip-relative lea)
    code[p]=0x4c; code[p+1]=0x8d; code[p+2]=0x1d; p+=3;                    // lea r11, [rip+disp32]
    let lea_patch = p; p+=4;                                                 // disp32 placeholder

    code[p]=0x4c; code[p+1]=0x89; code[p+2]=0x1c; code[p+3]=0x24; p+=4; // mov [rsp], r11

    // Set ALLOW
    code[p]=0x49; code[p+1]=0xbb; p+=2;
    code[p..p+8].copy_from_slice(&spa); p+=8;
    code[p]=0x4d; code[p+1]=0x8b; code[p+2]=0x1b; p+=3; // mov r11,[r11]
    code[p]=0x4d; code[p+1]=0x85; code[p+2]=0xdb; p+=3; // test r11,r11
    code[p]=0x74; code[p+1]=0x04; p+=2;                  // jz +4
    code[p]=0x41; code[p+1]=0xc6; code[p+2]=0x03; code[p+3]=0x00; p+=4;

    // jmp target
    code[p]=0x49; code[p+1]=0xba; p+=2;
    code[p..p+8].copy_from_slice(&tgt); p+=8;
    code[p]=0x41; code[p+1]=0xff; code[p+2]=0xe2; p+=3; // jmp r10

    // --- POST --- (target `ret`s here)
    let post_offset = p;
    // Patch lea displacement
    let rel = (post_offset as i32) - (lea_patch as i32 + 4);
    code[lea_patch..lea_patch+4].copy_from_slice(&rel.to_le_bytes());

    // Set BLOCK (use r11 — callee already returned so we can clobber it)
    code[p]=0x49; code[p+1]=0xbb; p+=2;
    code[p..p+8].copy_from_slice(&spa); p+=8;
    code[p]=0x4d; code[p+1]=0x8b; code[p+2]=0x1b; p+=3;
    code[p]=0x4d; code[p+1]=0x85; code[p+2]=0xdb; p+=3;
    code[p]=0x74; code[p+1]=0x04; p+=2;
    code[p]=0x41; code[p+1]=0xc6; code[p+2]=0x03; code[p+3]=0x01; p+=4;

    // jmp to saved ret addr: movabs r11, &SAVED_RET; mov r11, [r11]; jmp r11
    code[p]=0x49; code[p+1]=0xbb; p+=2;
    code[p..p+8].copy_from_slice(&sra); p+=8;
    code[p]=0x4d; code[p+1]=0x8b; code[p+2]=0x1b; p+=3; // mov r11, [r11]
    code[p]=0x41; code[p+1]=0xff; code[p+2]=0xe3; p+=3; // jmp r11

    assert!(p <= TRAMP_SIZE, "trampoline too large: {p} > {TRAMP_SIZE}");
    unsafe { std::ptr::copy_nonoverlapping(code.as_ptr(), tramp, p) };
    tramp as u64
}

pub fn default_registry() -> HashMap<String, HashMap<String, u64>> {
    let mut registry = HashMap::new();
    let mut s: HashMap<String, u64> = HashMap::new();

    macro_rules! reg {
        ($name:expr, $fn:expr) => { s.insert($name.into(), $fn as *const () as u64); };
    }

    macro_rules! reg_libc {
        ($name:expr, $fn:expr) => {{
            let addr = $fn as *const () as u64;
            if addr == 0 { log::warn!("reg_libc: {} resolved to NULL!", $name); }
            s.insert($name.into(), gen_trampoline(addr));
        }};
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
    reg_libc!("_exit", libc::exit);
    reg!("__exit", shim_exit);
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
    reg_libc!("_malloc", libc::malloc);
    reg_libc!("_free", libc::free);
    reg_libc!("_calloc", libc::calloc);
    reg_libc!("_realloc", libc::realloc);

    // String/memory
    reg_libc!("_strlen", libc::strlen);
    reg_libc!("_memcpy", libc::memcpy);
    reg_libc!("_memmove", libc::memmove);
    reg_libc!("_memset", libc::memset);
    reg_libc!("_memcmp", libc::memcmp);
    reg_libc!("_strcmp", libc::strcmp);
    reg_libc!("_strncmp", libc::strncmp);
    reg_libc!("_strcpy", libc::strcpy);
    reg_libc!("_strncpy", libc::strncpy);
    reg_libc!("___bzero", bzero);
    reg_libc!("_bzero", bzero);
    reg_libc!("_strchr", libc::strchr);
    reg_libc!("_strrchr", libc::strrchr);
    reg_libc!("_strstr", libc::strstr);
    reg_libc!("_strpbrk", libc::strpbrk);
    reg_libc!("_strspn", libc::strspn);
    reg_libc!("_strcspn", libc::strcspn);
    reg_libc!("_strerror", libc::strerror);
    reg_libc!("_strcoll", libc::strcoll);
    reg_libc!("_strtod", libc::strtod);
    reg_libc!("_memchr", libc::memchr);

    // Smart stdio shims
    reg!("_puts", shim_puts);
    reg_libc!("_printf", libc::printf);
    reg_libc!("_fprintf", libc::fprintf);
    reg_libc!("_fwrite", libc::fwrite);
    reg_libc!("_fread", libc::fread);
    reg_libc!("_fflush", libc::fflush);
    reg_libc!("_fclose", libc::fclose);
    reg_libc!("_fputc", libc::fputc);
    reg_libc!("_fputs", libc::fputs);
    reg_libc!("_getc", libc::fgetc);
    reg_libc!("___srget", libc::fgetc);
    reg_libc!("_ungetc", libc::ungetc);
    reg_libc!("_feof", libc::feof);
    reg_libc!("_ferror", libc::ferror);
    reg_libc!("_clearerr", libc::clearerr);

    reg_libc!("_fopen", libc::fopen);
    reg_libc!("_setvbuf", libc::setvbuf);
    reg_libc!("_setbuf", libc::setbuf);
    reg_libc!("_freopen", libc::freopen);
    reg_libc!("_ftell", libc::ftell);
    reg_libc!("_fseek", libc::fseek);
    reg_libc!("_rewind", libc::rewind);
    reg_libc!("_sscanf", libc::sscanf);
    reg_libc!("_fscanf", libc::fscanf);
    reg!("_snprintf", shim_snprintf_vararg_fix);
    reg_libc!("_sprintf", libc::sprintf);

    // time / locale / env
    reg_libc!("_time", libc::time);
    reg_libc!("_clock", clock);
    reg_libc!("_difftime", libc::difftime);
    reg_libc!("_mktime", libc::mktime);
    reg_libc!("_gmtime_r", libc::gmtime_r);
    reg_libc!("_localtime_r", libc::localtime_r);
    reg_libc!("_strftime", libc::strftime);
    reg_libc!("_setlocale", libc::setlocale);
    reg_libc!("_localeconv", libc::localeconv);
    reg_libc!("_getenv", libc::getenv);
    reg_libc!("_system", libc::system);

    // Math
    reg_libc!("_acos", acos);
    reg_libc!("_asin", asin);
    reg_libc!("_atan", atan);
    reg_libc!("_atan2", atan2);
    reg_libc!("_cos", cos);
    reg_libc!("_sin", sin);
    reg_libc!("_tan", tan);
    reg_libc!("_exp", exp);
    reg_libc!("_log", c_log);
    reg_libc!("_log10", log10);
    reg_libc!("_log2", log2);
    reg_libc!("_pow", pow);
    reg_libc!("_fmod", fmod);
    reg_libc!("_frexp", frexp);
    reg_libc!("_ldexp", ldexp);
    reg_libc!("_acosh", acosh);
    reg_libc!("_asinh", asinh);
    reg_libc!("_atanh", atanh);
    reg_libc!("_cbrt", cbrt);
    reg_libc!("_cosh", cosh);
    reg_libc!("_sinh", sinh);
    reg_libc!("_tanh", tanh);
    reg_libc!("_erf", erf);
    reg_libc!("_erfc", erfc);
    reg_libc!("_exp2", exp2);
    reg_libc!("_expm1", expm1);
    reg_libc!("_fdim", fdim);
    reg_libc!("_fma", fma);
    reg_libc!("_hypot", hypot);
    reg_libc!("_j0", j0);
    reg_libc!("_j1", j1);
    reg_libc!("_jn", jn);
    reg_libc!("_lgamma", lgamma);
    reg_libc!("_lgamma_r", lgamma_r);
    reg_libc!("_log1p", log1p);
    reg_libc!("_logb", logb);
    reg_libc!("_modf", modf);
    reg_libc!("_nextafter", nextafter);
    reg_libc!("_nexttoward", nexttoward);
    reg_libc!("_remainder", remainder);
    reg_libc!("_scalb", scalb);
    reg_libc!("_scalbln", scalbln);
    reg_libc!("_tgamma", tgamma);
    reg_libc!("_y0", y0);
    reg_libc!("_y1", y1);
    reg_libc!("_yn", yn);
    reg!("___exp10", shim_exp10);

    // Float variants — declared in our extern block, not in libc crate
    reg_libc!("_sqrtf", sqrtf);
    reg_libc!("_sqrt", sqrt);
    reg_libc!("_floorf", floorf);
    reg_libc!("_floor", floor);
    reg_libc!("_ceilf", ceilf);
    reg_libc!("_ceil", ceil);
    reg_libc!("_cosf", cosf);
    reg_libc!("_sinf", sinf);
    reg_libc!("_tanf", tanf);
    reg_libc!("_expf", expf);
    reg_libc!("_logf", logf);
    reg_libc!("_fmodf", fmodf);
    reg_libc!("_fmaxf", fmaxf);
    reg_libc!("_fminf", fminf);
    reg_libc!("_fmaxl", fmaxl);
    reg_libc!("_fabsf", fabsf);
    reg_libc!("_fabs", fabs);
    reg_libc!("_roundf", roundf);
    reg_libc!("_round", round);
    reg_libc!("_truncf", truncf);
    reg_libc!("_trunc", trunc);

    // Fortified functions — _chk variants have extra args, we forward to the base
    reg_libc!("___snprintf_chk", __snprintf_chk);
    reg_libc!("___sprintf_chk", __sprintf_chk);
    reg!("___memcpy_chk", shim_memcpy_chk);
    reg!("___memmove_chk", shim_memmove_chk);

    // OS locks (zig malloc uses these) — stub as no-ops for single-threaded
    reg!("_os_unfair_lock_lock", shim_noop);
    reg!("_os_unfair_lock_unlock", shim_noop);

    // Misc missing
    reg_libc!("_openat", libc::openat);
    reg_libc!("_readv", libc::readv);
    reg_libc!("_writev", libc::writev);
    reg_libc!("_msync", libc::msync);
    reg_libc!("_pwrite", libc::pwrite);
    reg!("_close$NOCANCEL", shim_close);
    reg!("_getcontext", shim_noop); // stub
    reg!("__availability_version_check", shim_noop_true);
    reg!("__dyld_image_count", shim_noop);
    reg!("__dyld_get_image_header", shim_noop);
    reg!("__dyld_get_image_name", shim_noop);
    reg!("__dyld_get_image_vmaddr_slide", shim_noop);
    reg!("_pthread_threadid_np", shim_noop);

    // String/memory extras
    reg_libc!("_strdup", libc::strdup);
    reg_libc!("_memmem", memmem);
    reg_libc!("_qsort", libc::qsort);
    reg_libc!("_atoi", libc::atoi);
    reg_libc!("_dirname", libc::dirname);
    reg_libc!("_realpath", libc::realpath);
    reg_libc!("_perror", libc::perror);
    reg_libc!("_putchar", libc::putchar);
    reg_libc!("_raise", libc::raise);
    reg_libc!("_rand", libc::rand);
    reg_libc!("_vsnprintf", vsnprintf);
    reg_libc!("_fgets", libc::fgets);
    reg_libc!("_fileno", libc::fileno);
    reg_libc!("_gettimeofday", libc::gettimeofday);
    reg_libc!("_pathconf", libc::pathconf);
    reg_libc!("_strptime", libc::strptime);
    reg_libc!("_timegm", timegm);
    reg_libc!("_fdopen", libc::fdopen);

    // $DARWIN_EXTSN and $INODE64 variants — same as base on Linux
    reg_libc!("_fopen$DARWIN_EXTSN", libc::fopen);
    reg_libc!("_fdopen$DARWIN_EXTSN", libc::fdopen);
    reg_libc!("_realpath$DARWIN_EXTSN", libc::realpath);
    reg_libc!("_stat$INODE64", libc::stat);
    reg_libc!("_fstat$INODE64", libc::fstat);

    // pthread — must wrap with selector toggle since host libc makes syscalls internally
    reg!("_pthread_create", shim_pthread_create);
    reg!("_pthread_join", shim_pthread_join);
    reg!("_pthread_getspecific", shim_pthread_getspecific);
    reg!("_pthread_setspecific", shim_pthread_setspecific);
    reg!("_pthread_key_create", shim_pthread_key_create);
    reg!("_pthread_once", shim_pthread_once);

    // CRT
    reg!("____chkstk_darwin", shim_noop);
    reg!("___assert_rtn", shim_assert_rtn);
    reg!("_memset_pattern16", shim_memset_pattern16);
    reg!("___error", shim_errno_location);
    reg!("___stack_chk_fail", shim_stack_chk_fail);
    reg!("_abort", shim_abort);
    reg_libc!("_atexit", libc::atexit);

    unsafe { setup_darwin_stdio(); }

    s.insert("_environ".into(), (&raw const ENVIRON_PTR) as u64);
    s.insert("___progname".into(), (&raw const PROGNAME_PTR) as u64);
    s.insert("___stack_chk_guard".into(), (&raw const STACK_CHK_GUARD) as u64);
    s.insert("_NXArgc".into(), (&raw const NXARGC) as u64);
    s.insert("_NXArgv".into(), (&raw const NXARGV) as u64);

    s.insert("___stdinp".into(), (&raw mut REAL_STDIN) as u64);
    s.insert("___stdoutp".into(), (&raw mut REAL_STDOUT) as u64);
    s.insert("___stderrp".into(), (&raw mut REAL_STDERR) as u64);
    init_rune_locale();
    s.insert("__DefaultRuneLocale".into(), (&raw mut DEFAULT_RUNE_LOCALE) as u64);
    
    reg!("___maskrune", shim_maskrune);
    reg_libc!("___tolower", libc::tolower);
    reg_libc!("___toupper", libc::toupper);
    reg_libc!("_isdigit", libc::isdigit);
    reg_libc!("_isalpha", libc::isalpha);
    reg_libc!("_isspace", libc::isspace);
    reg_libc!("_isupper", libc::isupper);
    reg_libc!("_islower", libc::islower);
    reg_libc!("_isprint", libc::isprint);
    reg_libc!("_iscntrl", libc::iscntrl);
    reg_libc!("_ispunct", libc::ispunct);
    reg_libc!("_isxdigit", libc::isxdigit);
    reg_libc!("_isalnum", libc::isalnum);
    reg_libc!("_toupper", libc::toupper);
    reg_libc!("_tolower", libc::tolower);
    reg_libc!("___memcpy_chk", libc::memcpy);
    reg_libc!("___strcpy_chk", libc::strcpy);
    reg_libc!("_dlopen", libc::dlopen);
    reg_libc!("_dlclose", libc::dlclose);
    reg_libc!("_dlsym", libc::dlsym);
    reg_libc!("_dlerror", libc::dlerror);

    reg!("_sigaction", shim_noop); // stub
    reg!("__setjmp", _setjmp);
    reg!("__longjmp", _longjmp);

    for name in [
        "/usr/lib/libSystem.B.dylib",
        "/usr/lib/libSystem.dylib",
        "libSystem.B.dylib",
        "libSystem.dylib",
    ] {
        registry.insert(name.into(), s.clone());
    }

    let mut objc = HashMap::new();
    objc.insert("_objc_msgSend".into(), grafted_objc::objc_msgSend as *const () as u64);
    objc.insert("_objc_getClass".into(), grafted_objc::objc_getClass as *const () as u64);
    objc.insert("_sel_registerName".into(), grafted_objc::sel_registerName as *const () as u64);
    for name in ["/usr/lib/libobjc.A.dylib", "libobjc.A.dylib", "libobjc.dylib"] {
        registry.insert(name.into(), objc.clone());
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
}

// ---- Shim implementations ----

// Vararg ABI bridge: if caller set al=0 (no XMM args, Darwin style with
// doubles in integer regs), copy rcx/r8/r9 → xmm0/1/2.
// If al>0 (caller already set XMM), leave them alone.
// Then set al=8 so glibc reads from XMM save area.
#[unsafe(naked)]
unsafe extern "C" fn shim_snprintf_vararg_fix() {
    std::arch::naked_asm!(
        "test %al, %al",
        "jnz 1f",
        "movq %rcx, %xmm0",
        "movq %r8, %xmm1",
        "movq %r9, %xmm2",
        "1:",
        "mov $8, %al",
        "jmp {snprintf}",
        snprintf = sym libc::snprintf,
        options(att_syntax),
    );
}

unsafe extern "C" fn shim_noop() {}
unsafe extern "C" fn shim_noop_true() -> i32 { 1 }

// Darwin's ___maskrune(c, mask) — character classification.
// Returns mask & runetype[c]. We implement using Linux libc's is* functions.
// Darwin mask bits: _A=0x100, _C=0x200, _D=0x400, _L=0x2000, _P=0x10000,
// _S=0x4000, _U=0x8000, _X=0x10000, _B=0x20000, _R=0x40000
unsafe extern "C" fn shim_maskrune(c: i32, mask: u64) -> i32 {
    let mut result: u64 = 0;
    let uc = c as u32;
    if uc < 128 {
        if unsafe { libc::isalpha(c) } != 0 { result |= 0x00000100; } // _CTYPE_A
        if unsafe { libc::iscntrl(c) } != 0 { result |= 0x00000200; } // _CTYPE_C
        if unsafe { libc::isdigit(c) } != 0 { result |= 0x00000400; } // _CTYPE_D
        if unsafe { libc::islower(c) } != 0 { result |= 0x00002000; } // _CTYPE_L
        if unsafe { libc::isupper(c) } != 0 { result |= 0x00008000; } // _CTYPE_U
        if unsafe { libc::isspace(c) } != 0 { result |= 0x00004000; } // _CTYPE_S
        if unsafe { libc::ispunct(c) } != 0 { result |= 0x00010000; } // _CTYPE_P
        if unsafe { libc::isxdigit(c) } != 0 { result |= 0x00100000; } // _CTYPE_X
        if unsafe { libc::isblank(c) } != 0 { result |= 0x00020000; } // _CTYPE_B
        if unsafe { libc::isprint(c) } != 0 { result |= 0x00040000; } // _CTYPE_R
    }
    (result & mask) as i32
}

// __snprintf_chk(buf, maxlen, flag, real_maxlen, fmt, ...) → snprintf(buf, maxlen, fmt, ...)
// We ignore flag and real_maxlen, just forward to host snprintf
unsafe extern "C" fn shim_snprintf_chk(
    buf: *mut i8, maxlen: usize, _flag: i32, _real_maxlen: usize,
    fmt: *const i8, a1: u64, a2: u64, a3: u64, a4: u64,
) -> i32 {
    selector_allow();
    let ret = unsafe { libc::snprintf(buf, maxlen, fmt, a1, a2, a3, a4) };
    selector_block();
    ret
}

// __memcpy_chk(dst, src, len, dst_len) → memcpy(dst, src, len)
unsafe extern "C" fn shim_memcpy_chk(
    dst: *mut libc::c_void, src: *const libc::c_void, len: usize, _dst_len: usize,
) -> *mut libc::c_void {
    unsafe { libc::memcpy(dst, src, len) }
}

// __sprintf_chk(buf, flag, maxlen, fmt, ...) → sprintf(buf, fmt, ...)
unsafe extern "C" fn shim_sprintf_chk(
    buf: *mut i8, _flag: i32, _maxlen: usize,
    fmt: *const i8, a1: u64, a2: u64, a3: u64, a4: u64,
) -> i32 {
    selector_allow();
    let ret = unsafe { libc::sprintf(buf, fmt, a1, a2, a3, a4) };
    selector_block();
    ret
}

// __memmove_chk(dst, src, len, dst_len) → memmove(dst, src, len)
unsafe extern "C" fn shim_memmove_chk(
    dst: *mut libc::c_void, src: *const libc::c_void, len: usize, _dst_len: usize,
) -> *mut libc::c_void {
    unsafe { libc::memmove(dst, src, len) }
}

unsafe extern "C" fn shim_exp10(x: f64) -> f64 {
    unsafe { pow(10.0, x) }
}

static mut ERRNO_VALUE: i32 = 0;
unsafe extern "C" fn shim_errno_location() -> *mut i32 {
    (&raw mut ERRNO_VALUE) as *mut i32
}

unsafe extern "C" fn shim_abort() -> ! {
    selector_allow();
    unsafe { libc::_exit(134) };
}

// pthread TLS — Darwin uses 64-bit pthread_key_t, Linux uses 32-bit.
// We use in-process TLS with 64-bit keys to avoid the ABI mismatch.
static TLS_STORE: std::sync::Mutex<Option<HashMap<u64, u64>>> = std::sync::Mutex::new(None);
static TLS_NEXT_KEY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

unsafe extern "C" fn shim_pthread_key_create(key_out: *mut u64, _dtor: Option<unsafe extern "C" fn(*mut libc::c_void)>) -> i32 {
    let key = TLS_NEXT_KEY.fetch_add(1, Ordering::Relaxed);
    unsafe { *key_out = key };
    0
}
unsafe extern "C" fn shim_pthread_setspecific(key: u64, val: *const libc::c_void) -> i32 {
    let mut guard = TLS_STORE.lock().unwrap();
    let map = guard.get_or_insert_with(HashMap::new);
    map.insert(key, val as u64);
    0
}
unsafe extern "C" fn shim_pthread_getspecific(key: u64) -> *mut libc::c_void {
    let guard = TLS_STORE.lock().unwrap();
    guard.as_ref()
        .and_then(|m| m.get(&key))
        .map(|&v| v as *mut libc::c_void)
        .unwrap_or(std::ptr::null_mut())
}
unsafe extern "C" fn shim_pthread_create(
    thread: *mut libc::pthread_t,
    attr: *const libc::pthread_attr_t,
    start: extern "C" fn(*mut libc::c_void) -> *mut libc::c_void,
    arg: *mut libc::c_void,
) -> i32 {
    selector_allow();
    let ret = unsafe { libc::pthread_create(thread, attr, start, arg) };
    selector_block();
    ret
}
unsafe extern "C" fn shim_pthread_join(thread: libc::pthread_t, retval: *mut *mut libc::c_void) -> i32 {
    selector_allow();
    let ret = unsafe { libc::pthread_join(thread, retval) };
    selector_block();
    ret
}
unsafe extern "C" fn shim_pthread_once(once: *mut libc::pthread_once_t, init: extern "C" fn()) -> i32 {
    selector_allow();
    let ret = unsafe { libc::pthread_once(once, init) };
    selector_block();
    ret
}

unsafe extern "C" fn shim_stack_chk_fail() -> ! {
    let msg = b"*** stack smashing detected ***\n";
    unsafe { libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len()) };
    unsafe { libc::_exit(134) };
}

unsafe extern "C" fn shim_assert_rtn(func: *const i8, file: *const i8, line: i32, expr: *const i8) -> ! {
    let msg = b"Assertion failed\n";
    unsafe { libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len()) };
    let _ = (func, file, line, expr);
    unsafe { libc::_exit(134) };
}

unsafe extern "C" fn shim_memset_pattern16(dst: *mut u8, pattern: *const u8, count: usize) {
    let mut i = 0;
    while i < count {
        let chunk = if count - i >= 16 { 16 } else { count - i };
        unsafe { std::ptr::copy_nonoverlapping(pattern, dst.add(i), chunk) };
        i += 16;
    }
}

unsafe extern "C" fn shim_exit(code: i32) -> ! {
    // Flush all stdio before exiting (Darwin exit() does this, _exit() doesn't)
    selector_allow();
    unsafe { libc::fflush(std::ptr::null_mut()) };
    selector_block();
    linux_syscall!(60, code);
    unreachable!()
}

unsafe extern "C" fn shim_write(fd: i32, buf: *const u8, count: usize) -> i64 {
    linux_syscall!(1, fd, buf, count)
}

unsafe extern "C" fn shim_read(fd: i32, buf: *mut u8, count: usize) -> i64 {
    linux_syscall!(0, fd, buf, count)
}

unsafe extern "C" fn shim_open(path: *const i8, flags: i32, mode: i32) -> i64 {
    linux_syscall!(2, path, flags, mode)
}

unsafe extern "C" fn shim_close(fd: i32) -> i64 {
    linux_syscall!(3, fd)
}

unsafe extern "C" fn shim_lseek(fd: i32, offset: i64, whence: i32) -> i64 {
    linux_syscall!(8, fd, offset, whence)
}

unsafe extern "C" fn shim_dup(fd: i32) -> i64 {
    linux_syscall!(32, fd)
}

unsafe extern "C" fn shim_dup2(fd: i32, fd2: i32) -> i64 {
    linux_syscall!(33, fd, fd2)
}

unsafe extern "C" fn shim_fcntl(fd: i32, cmd: i32, arg: u64) -> i64 {
    linux_syscall!(72, fd, cmd, arg)
}

unsafe extern "C" fn shim_isatty(fd: i32) -> i32 {
    let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
    let ret = linux_syscall!(16, fd, 0x5401, termios.as_mut_ptr()); // TCGETS
    if ret == 0 { 1 } else { 0 }
}

unsafe extern "C" fn shim_getpid() -> i32 {
    linux_syscall!(39, 0) as i32
}

unsafe extern "C" fn shim_getuid() -> i32 {
    linux_syscall!(102, 0) as i32
}

unsafe extern "C" fn shim_geteuid() -> i32 {
    linux_syscall!(107, 0) as i32
}

unsafe extern "C" fn shim_getgid() -> i32 {
    linux_syscall!(104, 0) as i32
}

unsafe extern "C" fn shim_getegid() -> i32 {
    linux_syscall!(108, 0) as i32
}

unsafe extern "C" fn shim_mmap(addr: u64, len: usize, prot: i32, flags: i32, fd: i32, offset: i64) -> u64 {
    selector_allow();
    let ret = unsafe { libc::mmap(addr as *mut _, len, prot, flags, fd, offset) };
    selector_block();
    ret as u64
}

unsafe extern "C" fn shim_munmap(addr: u64, len: usize) -> i32 {
    selector_allow();
    let ret = unsafe { libc::munmap(addr as *mut _, len) };
    selector_block();
    ret
}

unsafe extern "C" fn shim_mprotect(addr: u64, len: usize, prot: i32) -> i32 {
    selector_allow();
    let ret = unsafe { libc::mprotect(addr as *mut _, len, prot) };
    selector_block();
    ret
}

unsafe extern "C" fn shim_fstat(fd: i32, darwin_stat: *mut u8) -> i32 {
    let mut linux_stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    let ret = linux_syscall!(5, fd, linux_stat.as_mut_ptr());
    if ret == 0 {
        let linux_stat = unsafe { linux_stat.assume_init() };
        unsafe {
            std::ptr::write_bytes(darwin_stat, 0, 144);
            *(darwin_stat.add(0) as *mut i32) = linux_stat.st_dev as i32;
            *(darwin_stat.add(8) as *mut u64) = linux_stat.st_ino;
            *(darwin_stat.add(16) as *mut u16) = linux_stat.st_mode as u16;
            *(darwin_stat.add(48) as *mut i64) = linux_stat.st_size;
        }
    }
    ret as i32
}

unsafe extern "C" fn shim_stat(path: *const i8, darwin_stat: *mut u8) -> i32 {
    let mut linux_stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    let ret = linux_syscall!(4, path, linux_stat.as_mut_ptr());
    if ret == 0 {
        let linux_stat = unsafe { linux_stat.assume_init() };
        unsafe {
            std::ptr::write_bytes(darwin_stat, 0, 144);
            *(darwin_stat.add(48) as *mut i64) = linux_stat.st_size;
        }
    }
    ret as i32
}

unsafe extern "C" fn shim_puts(s: *const i8) -> i32 {
    selector_allow();
    let ret = unsafe { libc::puts(s) };
    selector_block();
    ret
}

// Global data symbols
static mut ENVIRON_PTR: *const *const i8 = std::ptr::null();
static mut PROGNAME_PTR: *const i8 = std::ptr::null();
static mut STACK_CHK_GUARD: usize = 0xdeadbeef;
static mut NXARGC: i32 = 0;
static mut NXARGV: *const *const i8 = std::ptr::null();

