//! libSystem.B.dylib shim — in-process function table.
//!
//! Shim functions toggle the SUD selector byte (ALLOW before syscall, BLOCK after)
//! so that our Linux syscalls pass through while Darwin code remains intercepted.

use std::arch::asm;
use std::collections::HashMap;
use std::sync::atomic::Ordering;

const FILTER_ALLOW: u8 = grafted_loader::executor::SYSCALL_DISPATCH_FILTER_ALLOW;
const FILTER_BLOCK: u8 = grafted_loader::executor::SYSCALL_DISPATCH_FILTER_BLOCK;

/// Set process info globals: executable path + argc/argv for _NSGetArgc/v
pub fn set_process_info(binary_path: &str, argv: &[String]) {
    unsafe {
        let path_bytes = binary_path.as_bytes();
        let ep = &raw mut EXECUTABLE_PATH;
        let len = path_bytes.len().min(1023);
        std::ptr::copy_nonoverlapping(path_bytes.as_ptr(), (*ep).as_mut_ptr(), len);
        (*ep)[len] = 0;

        // Build a C-style argv array that persists (leaked intentionally)
        let mut c_argv: Vec<*const i8> = Vec::with_capacity(argv.len() + 1);
        for arg in argv {
            let cstr = std::ffi::CString::new(arg.as_str()).unwrap_or_default();
            c_argv.push(cstr.into_raw() as *const i8);
        }
        c_argv.push(std::ptr::null()); // NULL terminator

        let argv_ptr = c_argv.as_ptr();
        std::mem::forget(c_argv); // leak — must persist for process lifetime

        NXARGC = argv.len() as i32;
        NXARGV = argv_ptr;
    }
}

fn selector_allow() {
    let ptr = grafted_loader::executor::selector_ptr();
    if !ptr.is_null() {
        unsafe { ptr.write_volatile(FILTER_ALLOW) };
    }
}

fn selector_block() {
    let ptr = grafted_loader::executor::selector_ptr();
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
    pub fn __vsnprintf_chk(s: *mut i8, maxlen: libc::size_t, flag: libc::c_int, slen: libc::size_t, fmt: *const i8, ap: *mut libc::c_void) -> libc::c_int;
    pub fn __memset_chk(dest: *mut libc::c_void, c: libc::c_int, len: libc::size_t, destlen: libc::size_t) -> *mut libc::c_void;
    pub fn __strncpy_chk(dest: *mut i8, src: *const i8, len: libc::size_t, destlen: libc::size_t) -> *mut i8;
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

/// Generate an executable trampoline that wraps a libc function pointer:
///   save argument registers
///   call grafted_selector_allow()   ; per-thread ALLOW
///   restore argument registers
///   jmp <target_fn>                 ; tail call
/// Returns the trampoline's address. All trampolines are allocated from
/// a single mmap'd executable page pool.
fn gen_trampoline(target: u64) -> u64 {
    use std::sync::Mutex;
    struct PoolPtr(*mut u8);
    unsafe impl Send for PoolPtr {}
    static POOL: Mutex<Option<(PoolPtr, usize)>> = Mutex::new(None);
    const PAGE_SIZE: usize = 4096 * 16;
    const TRAMP_SIZE: usize = 48;

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

    // Per-thread ALLOW trampoline: call grafted_selector_allow() then jmp to target.
    // Saves/restores all argument registers so the libc function sees correct args.
    // 7 pushes for 16-byte stack alignment before the call.
    let afn = (grafted_loader::executor::grafted_selector_allow as *const () as u64).to_le_bytes();
    let tgt = target.to_le_bytes();

    let mut code = [0u8; 48];
    let mut p = 0;

    // Save argument registers (7 pushes = 56 bytes → aligns stack for call)
    code[p] = 0x50; p += 1;                       // push rax (al = SSE arg count for variadics)
    code[p] = 0x57; p += 1;                       // push rdi
    code[p] = 0x56; p += 1;                       // push rsi
    code[p] = 0x52; p += 1;                       // push rdx
    code[p] = 0x51; p += 1;                       // push rcx
    code[p] = 0x41; code[p+1] = 0x50; p += 2;    // push r8
    code[p] = 0x41; code[p+1] = 0x51; p += 2;    // push r9

    // call grafted_selector_allow (sets this thread's selector to ALLOW)
    code[p] = 0x48; code[p+1] = 0xb8; p += 2;    // movabs rax, imm64
    code[p..p+8].copy_from_slice(&afn); p += 8;
    code[p] = 0xff; code[p+1] = 0xd0; p += 2;    // call rax

    // Restore argument registers (reverse order)
    code[p] = 0x41; code[p+1] = 0x59; p += 2;    // pop r9
    code[p] = 0x41; code[p+1] = 0x58; p += 2;    // pop r8
    code[p] = 0x59; p += 1;                       // pop rcx
    code[p] = 0x5a; p += 1;                       // pop rdx
    code[p] = 0x5e; p += 1;                       // pop rsi
    code[p] = 0x5f; p += 1;                       // pop rdi
    code[p] = 0x58; p += 1;                       // pop rax

    // jmp target (tail call — perfect stack preservation)
    code[p] = 0x49; code[p+1] = 0xbb; p += 2;    // movabs r11, imm64
    code[p..p+8].copy_from_slice(&tgt); p += 8;
    code[p] = 0x41; code[p+1] = 0xff; code[p+2] = 0xe3; p += 3; // jmp r11

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
    reg_libc!("_getchar", libc::getchar);
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
    reg_libc!("___vsnprintf_chk", __vsnprintf_chk);
    reg_libc!("___memset_chk", __memset_chk);
    reg_libc!("___strncpy_chk", __strncpy_chk);
    reg!("___memcpy_chk", shim_memcpy_chk);
    reg!("___memmove_chk", shim_memmove_chk);

    // OS locks (zig malloc uses these) — stub as no-ops for single-threaded
    reg!("_os_unfair_lock_lock", shim_noop);
    reg!("_os_unfair_lock_unlock", shim_noop);

    // Misc missing
    reg!("_openat", shim_openat);
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

    // pthread — custom wrappers that toggle selector AND handle Darwin/Linux key size mismatch
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

    // _NSGet* functions — return pointers to the global CRT variables
    reg!("__NSGetArgc", shim_nsgetargc);
    reg!("__NSGetArgv", shim_nsgetargv);
    reg!("__NSGetEnviron", shim_nsgetenviron);
    reg!("__NSGetExecutablePath", shim_nsgetexecutablepath);

    // C++ exception unwinding — stub out for Rust panic=abort builds
    reg!("__Unwind_Backtrace", shim_noop);
    reg!("__Unwind_GetIP", shim_noop);
    reg!("__Unwind_GetRegionStart", shim_noop);
    reg!("__Unwind_SetGR", shim_noop);
    reg!("__Unwind_SetIP", shim_noop);
    reg!("__Unwind_RaiseException", shim_noop);
    reg!("__Unwind_Resume", shim_noop);
    reg!("__Unwind_DeleteException", shim_noop);
    reg!("__Unwind_GetDataRelBase", shim_noop);
    reg!("__Unwind_GetTextRelBase", shim_noop);
    reg!("__Unwind_FindEnclosingFunction", shim_noop);
    reg!("__Unwind_GetLanguageSpecificData", shim_noop);

    // mach_* stubs for Rust std
    reg!("_mach_task_self", shim_mach_task_self);
    reg!("_mach_thread_self", shim_mach_thread_self);
    reg!("_host_self", shim_host_self);
    reg!("_mach_host_self", shim_host_self);
    reg!("_mach_port_allocate", shim_mach_port_allocate);
    reg!("_mach_port_deallocate", shim_mach_port_deallocate);
    reg!("_mach_port_insert_right", shim_mach_port_insert_right);
    reg!("_mach_port_mod_refs", shim_mach_port_mod_refs);
    reg!("_mach_port_type", shim_mach_port_type);
    reg!("_mach_msg", shim_mach_msg);
    reg!("_mach_msg_overwrite", shim_mach_msg);
    reg!("_mach_reply_port", shim_mach_reply_port);
    reg!("_bootstrap_port", shim_bootstrap_port_addr);
    reg!("_mach_vm_protect", shim_mach_vm_protect);
    reg!("_mach_vm_map", shim_noop); // stub
    reg!("_vm_protect", shim_vm_protect);
    reg!("_vm_deallocate", shim_noop);

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

    reg!("_sigaction", shim_sigaction);
    reg_libc!("_signal", libc::signal);
    reg_libc!("_sigprocmask", libc::sigprocmask);
    reg_libc!("_sigemptyset", libc::sigemptyset);
    reg_libc!("_sigfillset", libc::sigfillset);
    reg_libc!("_sigaddset", libc::sigaddset);
    reg_libc!("_kill", libc::kill);

    // I/O multiplexing
    reg_libc!("_poll", libc::poll);
    reg_libc!("_select", libc::select);
    reg_libc!("_select$1050", libc::select);
    reg_libc!("_pselect", libc::pselect);

    // stat — Darwin struct layout differs from Linux; use wrappers that translate
    reg!("_stat", shim_stat);
    reg!("_fstat", shim_fstat);
    reg!("_lstat", shim_lstat);
    reg!("_stat$INODE64", shim_stat);
    reg!("_fstat$INODE64", shim_fstat);
    reg!("_lstat$INODE64", shim_lstat);

    // Additional commonly needed functions
    reg_libc!("_access", libc::access);
    reg_libc!("_unlink", libc::unlink);
    reg_libc!("_rename", libc::rename);
    reg_libc!("_mkdir", libc::mkdir);
    reg_libc!("_rmdir", libc::rmdir);
    reg_libc!("_getcwd", libc::getcwd);
    reg_libc!("_chdir", libc::chdir);
    reg_libc!("_pipe", libc::pipe);

    // Networking
    reg_libc!("_socket", libc::socket);
    reg_libc!("_connect", libc::connect);
    reg_libc!("_bind", libc::bind);
    reg_libc!("_listen", libc::listen);
    reg_libc!("_accept", libc::accept);
    reg_libc!("_send", libc::send);
    reg_libc!("_recv", libc::recv);
    reg_libc!("_sendto", libc::sendto);
    reg_libc!("_recvfrom", libc::recvfrom);
    reg_libc!("_setsockopt", libc::setsockopt);
    reg_libc!("_getsockopt", libc::getsockopt);
    reg!("_getaddrinfo", shim_getaddrinfo);
    reg!("_freeaddrinfo", shim_freeaddrinfo);
    reg_libc!("_gai_strerror", libc::gai_strerror);
    reg_libc!("_getnameinfo", libc::getnameinfo);
    reg_libc!("_shutdown", libc::shutdown);
    reg_libc!("_fork", libc::fork);
    reg_libc!("_execve", libc::execve);
    reg_libc!("_waitpid", libc::waitpid);
    reg_libc!("_sysconf", libc::sysconf);

    // Directory operations
    reg_libc!("_opendir", libc::opendir);
    reg_libc!("_opendir$INODE64", libc::opendir);
    reg_libc!("_readdir_r$INODE64", libc::readdir_r);
    reg_libc!("_closedir", libc::closedir);
    reg_libc!("_dirfd", libc::dirfd);

    // Time
    reg_libc!("_clock_gettime", libc::clock_gettime);
    reg_libc!("_nanosleep", libc::nanosleep);

    // Process/user
    reg_libc!("_execvp", libc::execvp);
    reg_libc!("_gethostname", libc::gethostname);
    reg_libc!("_getpwuid_r", libc::getpwuid_r);
    reg_libc!("_getentropy", libc::getentropy);
    reg_libc!("_setuid", libc::setuid);
    reg_libc!("_setgid", libc::setgid);
    reg_libc!("_setgroups", libc::setgroups);
    reg_libc!("_setpgid", libc::setpgid);
    reg_libc!("_uname", libc::uname);
    reg_libc!("_ioctl", libc::ioctl);
    reg_libc!("_sched_yield", libc::sched_yield);
    reg_libc!("_strerror_r", libc::strerror_r);
    reg_libc!("_posix_memalign", libc::posix_memalign);
    reg_libc!("_posix_madvise", libc::posix_madvise);
    reg!("_sigaltstack", shim_sigaltstack_noop);
    reg!("_sysctl", shim_sysctl);
    reg!("_sysctlbyname", shim_sysctlbyname);

    // pthread extras
    reg_libc!("_pthread_self", libc::pthread_self);
    reg_libc!("_pthread_attr_init", libc::pthread_attr_init);
    reg_libc!("_pthread_attr_destroy", libc::pthread_attr_destroy);
    reg_libc!("_pthread_attr_setstacksize", libc::pthread_attr_setstacksize);
    reg_libc!("_pthread_detach", libc::pthread_detach);
    reg_libc!("_pthread_mutex_init", libc::pthread_mutex_init);
    reg_libc!("_pthread_mutex_destroy", libc::pthread_mutex_destroy);
    reg_libc!("_pthread_mutex_lock", libc::pthread_mutex_lock);
    reg_libc!("_pthread_mutex_unlock", libc::pthread_mutex_unlock);
    reg_libc!("_pthread_mutex_trylock", libc::pthread_mutex_trylock);
    reg_libc!("_pthread_mutexattr_init", libc::pthread_mutexattr_init);
    reg_libc!("_pthread_mutexattr_destroy", libc::pthread_mutexattr_destroy);
    reg_libc!("_pthread_mutexattr_settype", libc::pthread_mutexattr_settype);
    reg!("_pthread_setname_np", shim_pthread_setname_np);
    reg!("_pthread_get_stackaddr_np", shim_pthread_get_stackaddr_np);
    reg!("_pthread_get_stacksize_np", shim_pthread_get_stacksize_np);

    // posix_spawn (used by Rust std::process::Command)
    reg_libc!("_posix_spawnp", libc::posix_spawnp);
    reg_libc!("_posix_spawnattr_init", libc::posix_spawnattr_init);
    reg_libc!("_posix_spawnattr_destroy", libc::posix_spawnattr_destroy);
    reg_libc!("_posix_spawnattr_setflags", libc::posix_spawnattr_setflags);
    reg_libc!("_posix_spawnattr_setpgroup", libc::posix_spawnattr_setpgroup);
    reg_libc!("_posix_spawnattr_setsigdefault", libc::posix_spawnattr_setsigdefault);
    reg_libc!("_posix_spawn_file_actions_init", libc::posix_spawn_file_actions_init);
    reg_libc!("_posix_spawn_file_actions_destroy", libc::posix_spawn_file_actions_destroy);
    reg_libc!("_posix_spawn_file_actions_adddup2", libc::posix_spawn_file_actions_adddup2);

    // Grand Central Dispatch — minimal stubs using POSIX semaphores
    reg!("_dispatch_semaphore_create", shim_dispatch_semaphore_create);
    reg!("_dispatch_semaphore_signal", shim_dispatch_semaphore_signal);
    reg!("_dispatch_semaphore_wait", shim_dispatch_semaphore_wait);
    reg!("_dispatch_time", shim_dispatch_time);
    reg!("_dispatch_release", shim_noop);

    // TLS bootstrap — Darwin Thread Local Variables
    reg!("__tlv_bootstrap", shim_tlv_bootstrap_asm);
    reg!("__tlv_atexit", shim_noop);

    // More Unwind stubs
    reg!("__Unwind_GetCFA", shim_noop);
    reg!("__Unwind_GetIPInfo", shim_noop);
    reg_libc!("_getrlimit", libc::getrlimit);
    reg_libc!("_madvise", libc::madvise);
    reg_libc!("_srand", libc::srand);
    reg_libc!("_strtol", libc::strtol);
    reg_libc!("_strtoul", libc::strtoul);
    reg_libc!("_strtoll", libc::strtoll);
    reg_libc!("_strnlen", libc::strnlen);
    reg_libc!("_strncat", libc::strncat);
    reg_libc!("_strtok", libc::strtok);
    reg_libc!("_strcasecmp", libc::strcasecmp);
    reg_libc!("_strncasecmp", libc::strncasecmp);
    reg_libc!("_utimes", libc::utimes);
    reg_libc!("_chmod", libc::chmod);
    reg_libc!("_fsync", libc::fsync);
    reg_libc!("_link", libc::link);
    reg_libc!("_pread", libc::pread);
    reg_libc!("_readlink", libc::readlink);
    reg_libc!("_getpgid", libc::getpgid);
    reg_libc!("_getppid", libc::getppid);
    reg_libc!("_getsid", libc::getsid);
    reg_libc!("_pthread_rwlock_init", libc::pthread_rwlock_init);
    reg_libc!("_pthread_rwlock_destroy", libc::pthread_rwlock_destroy);
    reg_libc!("_pthread_rwlock_rdlock", libc::pthread_rwlock_rdlock);
    reg_libc!("_pthread_rwlock_wrlock", libc::pthread_rwlock_wrlock);
    reg_libc!("_pthread_rwlock_unlock", libc::pthread_rwlock_unlock);
    reg_libc!("_pthread_key_delete", libc::pthread_key_delete);
    reg!("_readdir$INODE64", shim_readdir);
    reg!("_mach_absolute_time", shim_mach_absolute_time);
    reg!("_mach_timebase_info", shim_mach_timebase_info);
    reg!("dyld_stub_binder", shim_noop);

    // Go runtime needs
    reg!("_stat64", shim_stat);
    reg!("_fstat64", shim_fstat);
    reg!("_lstat64", shim_lstat);
    reg_libc!("_fdopendir$INODE64", libc::fdopendir);
    reg_libc!("_chroot", libc::chroot);
    reg_libc!("_getpeername", libc::getpeername);
    reg_libc!("_getsockname", libc::getsockname);
    reg!("_kevent", shim_kevent);
    reg!("_kqueue", shim_kqueue);
    reg_libc!("_mkfifo", libc::mkfifo);
    reg_libc!("_sendfile", libc::sendfile);
    reg_libc!("_setrlimit", libc::setrlimit);
    reg_libc!("_setsid", libc::setsid);
    reg_libc!("_usleep", libc::usleep);
    reg_libc!("_wait4", libc::wait4);
    reg_libc!("_pthread_attr_getstacksize", libc::pthread_attr_getstacksize);
    reg_libc!("_pthread_attr_setdetachstate", libc::pthread_attr_setdetachstate);
    reg_libc!("_pthread_cond_init", libc::pthread_cond_init);
    reg_libc!("_pthread_cond_signal", libc::pthread_cond_signal);
    reg_libc!("_pthread_cond_wait", libc::pthread_cond_wait);
    reg_libc!("_pthread_kill", libc::pthread_kill);
    reg_libc!("_pthread_sigmask", libc::pthread_sigmask);
    reg!("_pthread_cond_timedwait_relative_np", shim_pthread_cond_timedwait_relative_np);
    reg!("_issetugid", shim_issetugid);
    reg!("_ptrace", shim_noop_ret0);
    reg!("_notify_is_valid_token", shim_noop_ret0);
    reg!("_xpc_date_create_from_current", shim_noop_ret0);
    reg!("__setjmp", _setjmp);
    reg!("__longjmp", _longjmp);

    // C++ new/delete (needed by Swift apps that import from "self")
    reg_libc!("__Znwm", libc::malloc);     // operator new(size_t)
    reg_libc!("__Znam", libc::malloc);     // operator new[](size_t)
    reg_libc!("__ZdlPv", libc::free);      // operator delete(void*)
    reg_libc!("__ZdaPv", libc::free);      // operator delete[](void*)

    for name in [
        "/usr/lib/libSystem.B.dylib",
        "/usr/lib/libSystem.dylib",
        "libSystem.B.dylib",
        "libSystem.dylib",
        "self", // Go/Swift binaries import from "self"
    ] {
        registry.insert(name.into(), s.clone());
    }

    let mut objc = HashMap::new();
    objc.insert("_objc_msgSend".into(), grafted_objc::objc_msgSend as *const () as u64);
    objc.insert("_objc_getClass".into(), grafted_objc::objc_getClass as *const () as u64);
    objc.insert("_objc_lookUpClass".into(), grafted_objc::objc_getClass as *const () as u64);
    objc.insert("_sel_registerName".into(), grafted_objc::sel_registerName as *const () as u64);
    objc.insert("_objc_registerClassPair".into(), grafted_objc::objc_registerClassPair as *const () as u64);
    // ARC: retain/release/autorelease
    objc.insert("_objc_retain".into(), shim_objc_retain as *const () as u64);
    objc.insert("_objc_release".into(), shim_objc_release as *const () as u64);
    objc.insert("_objc_retainAutorelease".into(), shim_objc_retain as *const () as u64);
    objc.insert("_objc_autoreleaseReturnValue".into(), shim_objc_retain as *const () as u64);
    objc.insert("_objc_retainAutoreleasedReturnValue".into(), shim_objc_retain as *const () as u64);
    objc.insert("_objc_allocWithZone".into(), shim_objc_alloc_with_zone as *const () as u64);
    objc.insert("_objc_opt_self".into(), shim_objc_retain as *const () as u64); // returns self
    objc.insert("_objc_msgSendSuper2".into(), shim_objc_msg_send_super2 as *const () as u64);
    objc.insert("_objc_msgSend_stret".into(), grafted_objc::objc_msgSend as *const () as u64);
    objc.insert("_objc_getAssociatedObject".into(), shim_noop_ret0 as *const () as u64);
    objc.insert("_objc_setAssociatedObject".into(), shim_noop as *const () as u64);
    objc.insert("_objc_setHook_getClass".into(), shim_noop as *const () as u64);
    // ObjC runtime globals
    objc.insert("__objc_empty_cache".into(),
        unsafe { &raw mut grafted_frameworks::registry::__objc_empty_cache } as u64);
    objc.insert("__objc_empty_vtable".into(),
        unsafe { &raw mut grafted_frameworks::registry::__objc_empty_vtable } as u64);
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
unsafe extern "C" fn shim_noop_ret0() -> i64 { 0 }

// ObjC ARC stubs
unsafe extern "C" fn shim_objc_retain(obj: *mut u8) -> *mut u8 { obj }
unsafe extern "C" fn shim_objc_release(_obj: *mut u8) {}
unsafe extern "C" fn shim_objc_alloc_with_zone(cls: *mut u8, _zone: *mut u8) -> *mut u8 {
    // Simplified alloc: calloc an instance
    let sel = grafted_objc::sel_registerName(b"alloc\0".as_ptr() as *const i8);
    grafted_objc::objc_msgSend(cls as *mut _, sel) as *mut u8
}
unsafe extern "C" fn shim_objc_msg_send_super2(
    super_: *mut u8, sel: *mut u8,
) -> *mut u8 {
    // Simplified: just dispatch on the receiver (super_->receiver)
    if super_.is_null() { return std::ptr::null_mut(); }
    let receiver = unsafe { *(super_ as *const *mut u8) };
    grafted_objc::objc_msgSend(receiver as *mut _, sel as *mut _) as *mut u8
}
unsafe extern "C" fn shim_issetugid() -> i32 { 0 } // not setuid

// kqueue/kevent — BSD-only, emulate with epoll
unsafe extern "C" fn shim_kqueue() -> i32 {
    selector_allow();
    let fd = unsafe { libc::epoll_create1(0) };
    selector_block();
    fd
}

unsafe extern "C" fn shim_kevent(
    kq: i32, changelist: *const u8, nchanges: i32,
    eventlist: *mut u8, nevents: i32, timeout: *const libc::timespec,
) -> i32 {
    // Minimal stub: if nevents > 0 and timeout is provided, do a timed wait via epoll
    if nevents > 0 {
        let timeout_ms = if timeout.is_null() {
            -1
        } else {
            let ts = unsafe { &*timeout };
            (ts.tv_sec * 1000 + ts.tv_nsec / 1_000_000) as i32
        };
        selector_allow();
        let mut ev: libc::epoll_event = unsafe { std::mem::zeroed() };
        let ret = unsafe { libc::epoll_wait(kq, &mut ev, 1, timeout_ms) };
        selector_block();
        return ret; // 0 = timeout, >0 = events
    }
    let _ = (changelist, nchanges, eventlist);
    0
}

// Darwin-specific pthread_cond_timedwait with relative timeout
unsafe extern "C" fn shim_pthread_cond_timedwait_relative_np(
    cond: *mut libc::pthread_cond_t,
    mutex: *mut libc::pthread_mutex_t,
    reltime: *const libc::timespec,
) -> i32 {
    if reltime.is_null() {
        selector_allow();
        let ret = unsafe { libc::pthread_cond_wait(cond, mutex) };
        selector_block();
        return ret;
    }
    // Convert relative timeout to absolute for Linux pthread_cond_timedwait
    let mut abstime: libc::timespec = unsafe { std::mem::zeroed() };
    selector_allow();
    unsafe { libc::clock_gettime(libc::CLOCK_REALTIME, &mut abstime) };
    let rel = unsafe { &*reltime };
    abstime.tv_sec += rel.tv_sec;
    abstime.tv_nsec += rel.tv_nsec;
    if abstime.tv_nsec >= 1_000_000_000 {
        abstime.tv_sec += 1;
        abstime.tv_nsec -= 1_000_000_000;
    }
    let ret = unsafe { libc::pthread_cond_timedwait(cond, mutex, &abstime) };
    selector_block();
    ret
}

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
// We bridge by zero-extending the key slot and toggling SUD selector
// around libc calls (which use futex internally).
unsafe extern "C" fn shim_pthread_key_create(key_out: *mut u64, dtor: Option<unsafe extern "C" fn(*mut libc::c_void)>) -> i32 {
    // Zero the 8-byte Darwin slot (Linux writes only 4 bytes for pthread_key_t)
    unsafe { *key_out = 0 };
    selector_allow();
    let ret = unsafe { libc::pthread_key_create(key_out as *mut libc::pthread_key_t, dtor) };
    selector_block();
    ret
}
unsafe extern "C" fn shim_pthread_setspecific(key: u64, val: *const libc::c_void) -> i32 {
    selector_allow();
    let ret = unsafe { libc::pthread_setspecific(key as libc::pthread_key_t, val) };
    selector_block();
    ret
}
unsafe extern "C" fn shim_pthread_getspecific(key: u64) -> *mut libc::c_void {
    selector_allow();
    let ret = unsafe { libc::pthread_getspecific(key as libc::pthread_key_t) };
    selector_block();
    ret
}
struct ThreadStartCtx {
    real_start: extern "C" fn(*mut libc::c_void) -> *mut libc::c_void,
    real_arg: *mut libc::c_void,
}
unsafe impl Send for ThreadStartCtx {}

extern "C" fn thread_start_wrapper(ctx_ptr: *mut libc::c_void) -> *mut libc::c_void {
    let ctx = unsafe { Box::from_raw(ctx_ptr as *mut ThreadStartCtx) };
    // Allocate a per-thread SUD selector byte and install SUD with correct range.
    // Each thread gets its own selector → no races with other threads.
    let sel_ptr = grafted_loader::executor::alloc_thread_selector();
    grafted_loader::executor::setup_thread_sud(sel_ptr);
    (ctx.real_start)(ctx.real_arg)
}

unsafe extern "C" fn shim_pthread_create(
    thread: *mut libc::pthread_t,
    attr: *const libc::pthread_attr_t,
    start: extern "C" fn(*mut libc::c_void) -> *mut libc::c_void,
    arg: *mut libc::c_void,
) -> i32 {
    selector_allow();
    let ctx = Box::into_raw(Box::new(ThreadStartCtx {
        real_start: start,
        real_arg: arg,
    }));
    let ret = unsafe { libc::pthread_create(thread, attr, thread_start_wrapper, ctx as *mut _) };
    selector_block();
    ret
}
unsafe extern "C" fn shim_pthread_join(thread: libc::pthread_t, retval: *mut *mut libc::c_void) -> i32 {
    selector_allow();
    let ret = unsafe { libc::pthread_join(thread, retval) };
    selector_block();
    ret
}
// Darwin PTHREAD_ONCE_INIT = {0x30B1BCBA, 0} (16 bytes).
// Linux PTHREAD_ONCE_INIT = 0 (4 bytes).
// We detect the Darwin magic and reset to Linux's 0 before calling.
const DARWIN_PTHREAD_ONCE_INIT: u32 = 0x30B1BCBA;

unsafe extern "C" fn shim_pthread_once(once: *mut libc::pthread_once_t, init: extern "C" fn()) -> i32 {
    // Check if this is an uninitialized Darwin pthread_once_t
    let once_val = unsafe { *(once as *const u32) };
    if once_val == DARWIN_PTHREAD_ONCE_INIT {
        // Reset to Linux PTHREAD_ONCE_INIT (0) so pthread_once will call init
        unsafe { *(once as *mut u32) = 0 };
    }
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

// Convert raw syscall return (negative = -errno) to libc convention (-1 + set errno)
fn syscall_ret(raw: i64) -> i64 {
    if raw < 0 {
        unsafe { *libc::__errno_location() = (-raw) as i32 };
        -1
    } else {
        raw
    }
}

unsafe extern "C" fn shim_write(fd: i32, buf: *const u8, count: usize) -> i64 {
    syscall_ret(linux_syscall!(1, fd, buf, count))
}

unsafe extern "C" fn shim_read(fd: i32, buf: *mut u8, count: usize) -> i64 {
    syscall_ret(linux_syscall!(0, fd, buf, count))
}

// Darwin open() flags differ from Linux:
//   Darwin O_CREAT=0x200, O_TRUNC=0x400, O_EXCL=0x800, O_APPEND=0x8, O_NONBLOCK=0x4
//   Linux  O_CREAT=0x40,  O_TRUNC=0x200, O_EXCL=0x80,  O_APPEND=0x400, O_NONBLOCK=0x800
fn translate_open_flags(darwin: i32) -> i32 {
    let mut linux = darwin & 0x3; // O_RDONLY=0, O_WRONLY=1, O_RDWR=2 — same on both
    if darwin & 0x0008 != 0 { linux |= 0x0400; } // O_APPEND
    if darwin & 0x0004 != 0 { linux |= 0x0800; } // O_NONBLOCK
    if darwin & 0x0200 != 0 { linux |= 0x0040; } // O_CREAT
    if darwin & 0x0400 != 0 { linux |= 0x0200; } // O_TRUNC
    if darwin & 0x0800 != 0 { linux |= 0x0080; } // O_EXCL
    if darwin & 0x0010 != 0 { linux |= 0x0100; } // O_NOCTTY (same? Darwin=0x20000 actually)
    if darwin & 0x10000 != 0 { linux |= 0x10000; } // O_DIRECTORY
    if darwin & 0x100000 != 0 { linux |= 0x200000; } // O_CLOEXEC
    linux
}

unsafe extern "C" fn shim_open(path: *const i8, flags: i32, mode: i32) -> i64 {
    let linux_flags = translate_open_flags(flags);
    syscall_ret(linux_syscall!(2, path, linux_flags, mode))
}

// Darwin vs Linux addrinfo: ai_canonname and ai_addr are swapped.
// Darwin: {flags, family, socktype, protocol, addrlen, *canonname, *addr, *next}
// Linux:  {flags, family, socktype, protocol, addrlen, *addr, *canonname, *next}
// We call Linux getaddrinfo then swap the two pointer fields in each result node.
unsafe extern "C" fn shim_getaddrinfo(
    node: *const i8, service: *const i8,
    hints: *const libc::addrinfo, res: *mut *mut libc::addrinfo,
) -> i32 {
    // If hints is non-null, it's in Darwin layout — but the first 5 fields (flags..addrlen)
    // are the same. The pointer fields in hints are typically NULL, so no swap needed.
    selector_allow();
    let ret = unsafe { libc::getaddrinfo(node, service, hints, res) };
    selector_block();
    if ret == 0 && !res.is_null() {
        // Walk the linked list and swap ai_addr <-> ai_canonname in each node
        let mut cur = unsafe { *res };
        while !cur.is_null() {
            unsafe {
                let addr = (*cur).ai_addr;
                let canon = (*cur).ai_canonname;
                // Swap: Darwin binary reads offset 24 as canonname, offset 32 as addr
                // Linux wrote addr at 24, canonname at 32
                // By swapping, Darwin binary finds addr at its expected offset 32
                (*cur).ai_addr = canon as *mut libc::sockaddr;
                (*cur).ai_canonname = addr as *mut i8;
                cur = (*cur).ai_next;
            }
        }
    }
    ret
}

unsafe extern "C" fn shim_freeaddrinfo(res: *mut libc::addrinfo) {
    if res.is_null() { return; }
    // Swap back before freeing so libc can correctly free the struct
    let mut cur = res;
    while !cur.is_null() {
        unsafe {
            let addr = (*cur).ai_addr;
            let canon = (*cur).ai_canonname;
            (*cur).ai_addr = canon as *mut libc::sockaddr;
            (*cur).ai_canonname = addr as *mut i8;
            cur = (*cur).ai_next;
        }
    }
    selector_allow();
    unsafe { libc::freeaddrinfo(res) };
    selector_block();
}

unsafe extern "C" fn shim_openat(dirfd: i32, path: *const i8, flags: i32, mode: i32) -> i64 {
    let linux_flags = translate_open_flags(flags);
    selector_allow();
    let ret = unsafe { libc::openat(dirfd, path, linux_flags, mode as libc::mode_t) } as i64;
    selector_block();
    ret
}

// Darwin struct stat (x86_64) — 144 bytes
// Linux fills a different layout; we translate field by field.
#[repr(C)]
struct DarwinStat {
    st_dev: i32,        // 0
    st_mode: u16,       // 4
    st_nlink: u16,      // 6
    st_ino: u64,        // 8
    st_uid: u32,        // 16
    st_gid: u32,        // 20
    st_rdev: i32,       // 24
    _pad0: i32,         // 28
    st_atim: [i64; 2],  // 32 (sec, nsec)
    st_mtim: [i64; 2],  // 48
    st_ctim: [i64; 2],  // 64
    st_birthtim: [i64; 2], // 80
    st_size: i64,       // 96
    st_blocks: i64,     // 104
    st_blksize: i32,    // 112
    st_flags: u32,      // 116
    st_gen: u32,        // 120
    _pad1: i32,         // 124
    _reserved: [i64; 2],// 128
}

fn linux_to_darwin_stat(linux: &libc::stat, darwin: *mut DarwinStat) {
    unsafe {
        (*darwin).st_dev = linux.st_dev as i32;
        (*darwin).st_mode = linux.st_mode as u16;
        (*darwin).st_nlink = linux.st_nlink as u16;
        (*darwin).st_ino = linux.st_ino;
        (*darwin).st_uid = linux.st_uid;
        (*darwin).st_gid = linux.st_gid;
        (*darwin).st_rdev = linux.st_rdev as i32;
        (*darwin)._pad0 = 0;
        (*darwin).st_atim = [linux.st_atime, linux.st_atime_nsec];
        (*darwin).st_mtim = [linux.st_mtime, linux.st_mtime_nsec];
        (*darwin).st_ctim = [linux.st_ctime, linux.st_ctime_nsec];
        (*darwin).st_birthtim = [linux.st_ctime, linux.st_ctime_nsec]; // Linux has no birthtime
        (*darwin).st_size = linux.st_size;
        (*darwin).st_blocks = linux.st_blocks;
        (*darwin).st_blksize = linux.st_blksize as i32;
        (*darwin).st_flags = 0;
        (*darwin).st_gen = 0;
        (*darwin)._pad1 = 0;
        (*darwin)._reserved = [0; 2];
    }
}

unsafe extern "C" fn shim_stat(path: *const i8, buf: *mut DarwinStat) -> i32 {
    let mut linux_buf: libc::stat = unsafe { std::mem::zeroed() };
    selector_allow();
    let ret = unsafe { libc::stat(path, &mut linux_buf) };
    selector_block();
    if ret == 0 { linux_to_darwin_stat(&linux_buf, buf); }
    ret
}

unsafe extern "C" fn shim_fstat(fd: i32, buf: *mut DarwinStat) -> i32 {
    let mut linux_buf: libc::stat = unsafe { std::mem::zeroed() };
    selector_allow();
    let ret = unsafe { libc::fstat(fd, &mut linux_buf) };
    selector_block();
    if ret == 0 { linux_to_darwin_stat(&linux_buf, buf); }
    ret
}

unsafe extern "C" fn shim_lstat(path: *const i8, buf: *mut DarwinStat) -> i32 {
    let mut linux_buf: libc::stat = unsafe { std::mem::zeroed() };
    selector_allow();
    let ret = unsafe { libc::lstat(path, &mut linux_buf) };
    selector_block();
    if ret == 0 { linux_to_darwin_stat(&linux_buf, buf); }
    ret
}

unsafe extern "C" fn shim_close(fd: i32) -> i64 {
    syscall_ret(linux_syscall!(3, fd))
}

unsafe extern "C" fn shim_lseek(fd: i32, offset: i64, whence: i32) -> i64 {
    syscall_ret(linux_syscall!(8, fd, offset, whence))
}

unsafe extern "C" fn shim_dup(fd: i32) -> i64 {
    syscall_ret(linux_syscall!(32, fd))
}

unsafe extern "C" fn shim_dup2(fd: i32, fd2: i32) -> i64 {
    syscall_ret(linux_syscall!(33, fd, fd2))
}

unsafe extern "C" fn shim_fcntl(fd: i32, cmd: i32, arg: u64) -> i64 {
    syscall_ret(linux_syscall!(72, fd, cmd, arg))
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

// Darwin mmap flags differ from Linux:
// Darwin MAP_ANON=0x1000, Linux MAP_ANON=0x20
// Darwin MAP_PRIVATE=0x02, Linux MAP_PRIVATE=0x02 (same)
// Darwin MAP_FIXED=0x10, Linux MAP_FIXED=0x10 (same)
// Darwin MAP_NORESERVE=0x40, Linux MAP_NORESERVE=0x4000
fn translate_mmap_flags(darwin: i32) -> i32 {
    let mut linux = darwin & 0x1F; // MAP_SHARED(1), MAP_PRIVATE(2), MAP_FIXED(0x10) — same
    if darwin & 0x1000 != 0 { linux |= 0x20; } // MAP_ANON
    if darwin & 0x0040 != 0 { linux |= 0x4000; } // MAP_NORESERVE
    linux
}

unsafe extern "C" fn shim_mmap(addr: u64, len: usize, prot: i32, flags: i32, fd: i32, offset: i64) -> u64 {
    let linux_flags = translate_mmap_flags(flags);

    // Guard page (PROT_NONE + MAP_FIXED + MAP_ANON) — fake success
    if prot == 0 && linux_flags & 0x30 == 0x30 {
        selector_allow();
        unsafe { libc::write(2, b"GUARD_FAKE\n".as_ptr() as *const _, 11) };
        selector_block();
        return if addr != 0 { addr } else { 0x7fff_dead_0000 };
    }
    if prot == 0 {
        selector_allow();
        unsafe { libc::write(2, b"MMAP_P0_OTHER\n".as_ptr() as *const _, 14) };
        selector_block();
    }

    // For MAP_FIXED with unaligned address: align down but return the ORIGINAL
    // requested address to the caller (Rust checks for exact match)
    let actual_addr = if linux_flags & 0x10 != 0 && addr & 0xFFF != 0 {
        addr & !0xFFF
    } else {
        addr
    };
    let actual_len = if actual_addr != addr {
        len + (addr - actual_addr) as usize
    } else {
        len
    };
    let return_original = linux_flags & 0x10 != 0 && addr != actual_addr;
    selector_allow();
    let ret = unsafe { libc::mmap(actual_addr as *mut _, actual_len, prot, linux_flags, fd, offset) };
    selector_block();
    if ret == libc::MAP_FAILED {
        return ret as u64;
    }
    // If we aligned down, return the original address the caller expected
    if return_original { addr } else { ret as u64 }
}

unsafe extern "C" fn shim_munmap(addr: u64, len: usize) -> i32 {
    selector_allow();
    let ret = unsafe { libc::munmap(addr as *mut _, len) };
    selector_block();
    ret
}

unsafe extern "C" fn shim_mprotect(addr: u64, len: usize, prot: i32) -> i32 {
    // PROT_NONE = guard page request — always fake success.
    if prot == 0 { return 0; }
    selector_allow();
    let ret = unsafe { libc::mprotect(addr as *mut _, len, prot) };
    selector_block();
    ret
}

// Darwin sigaction struct translation
// Darwin: { handler(8), sa_mask(4), sa_flags(4) } = 16 bytes
// Linux:  { handler(8), sa_flags(8), sa_restorer(8), sa_mask(128) } = 152 bytes
#[repr(C)]
struct DarwinSigaction {
    sa_handler: u64,
    sa_mask: u32,
    sa_flags: i32,
}

// Darwin SA_* flags that differ from Linux
const DARWIN_SA_SIGINFO: i32 = 0x0040;
const LINUX_SA_SIGINFO: i32 = 4;
const DARWIN_SA_RESTART: i32 = 0x0002;
const LINUX_SA_RESTART: i32 = 0x10000000;
const DARWIN_SA_NOCLDSTOP: i32 = 0x0008;
const LINUX_SA_NOCLDSTOP: i32 = 1;
const DARWIN_SA_NODEFER: i32 = 0x0010;
const LINUX_SA_NODEFER: i32 = 0x40000000;
const DARWIN_SA_ONSTACK: i32 = 0x0001;
const LINUX_SA_ONSTACK: i32 = 0x08000000;
const DARWIN_SA_RESETHAND: i32 = 0x0004;
const LINUX_SA_RESETHAND: i32 = 0x80000000_u32 as i32;

fn translate_sa_flags(darwin: i32) -> u64 {
    let mut linux: u64 = 0;
    if darwin & DARWIN_SA_SIGINFO != 0 { linux |= LINUX_SA_SIGINFO as u64; }
    if darwin & DARWIN_SA_RESTART != 0 { linux |= LINUX_SA_RESTART as u64; }
    if darwin & DARWIN_SA_NOCLDSTOP != 0 { linux |= LINUX_SA_NOCLDSTOP as u64; }
    if darwin & DARWIN_SA_NODEFER != 0 { linux |= LINUX_SA_NODEFER as u64; }
    if darwin & DARWIN_SA_ONSTACK != 0 { linux |= LINUX_SA_ONSTACK as u64; }
    if darwin & DARWIN_SA_RESETHAND != 0 { linux |= LINUX_SA_RESETHAND as u64; }
    linux
}

unsafe extern "C" fn shim_sigaction(sig: i32, act: *const DarwinSigaction, oldact: *mut DarwinSigaction) -> i32 {
    selector_allow();
    let mut linux_act: libc::sigaction = unsafe { std::mem::zeroed() };
    let mut linux_oldact: libc::sigaction = unsafe { std::mem::zeroed() };

    let act_ptr = if !act.is_null() {
        let da = unsafe { &*act };
        linux_act.sa_sigaction = da.sa_handler as usize;
        linux_act.sa_flags = translate_sa_flags(da.sa_flags) as i32;
        // Convert Darwin 32-bit mask to Linux 128-byte mask
        unsafe { libc::sigemptyset(&mut linux_act.sa_mask) };
        for bit in 0..32 {
            if da.sa_mask & (1 << bit) != 0 {
                unsafe { libc::sigaddset(&mut linux_act.sa_mask, bit + 1) };
            }
        }
        &linux_act as *const _
    } else {
        std::ptr::null()
    };

    let oldact_ptr = if !oldact.is_null() {
        &mut linux_oldact as *mut _
    } else {
        std::ptr::null_mut()
    };

    let ret = unsafe { libc::sigaction(sig, act_ptr, oldact_ptr) };

    if ret == 0 && !oldact.is_null() {
        unsafe {
            (*oldact).sa_handler = linux_oldact.sa_sigaction as u64;
            (*oldact).sa_flags = linux_oldact.sa_flags; // approximate
            (*oldact).sa_mask = 0; // simplified
        }
    }

    selector_block();
    ret
}

// sigaltstack stub — prevents Rust from failing during stack overflow handler setup
unsafe extern "C" fn shim_sigaltstack_noop(_ss: *const libc::stack_t, _oss: *mut libc::stack_t) -> i32 {
    if !_oss.is_null() {
        unsafe {
            (*_oss).ss_sp = std::ptr::null_mut();
            (*_oss).ss_flags = 2; // SS_DISABLE
            (*_oss).ss_size = 0;
        }
    }
    0
}

// Old stat shims removed — replaced by DarwinStat translation wrappers above

unsafe extern "C" fn shim_puts(s: *const i8) -> i32 {
    selector_allow();
    let ret = unsafe { libc::puts(s) };
    selector_block();
    ret
}

// Global data symbols
static mut ENVIRON_PTR: *const *const i8 = std::ptr::null();
static mut PROGNAME_PTR: *const i8 = std::ptr::null();
static mut EXECUTABLE_PATH: [u8; 1024] = [0; 1024];

// _NSGet* functions return pointers to the CRT globals
unsafe extern "C" fn shim_nsgetargc() -> *mut i32 {
    (&raw mut NXARGC) as *mut i32
}
unsafe extern "C" fn shim_nsgetargv() -> *mut *const *const i8 {
    (&raw mut NXARGV) as *mut *const *const i8
}
unsafe extern "C" fn shim_nsgetenviron() -> *mut *const *const i8 {
    (&raw mut ENVIRON_PTR) as *mut *const *const i8
}
unsafe extern "C" fn shim_nsgetexecutablepath(buf: *mut i8, bufsize: *mut u32) -> i32 {
    let path_ptr = (&raw const EXECUTABLE_PATH) as *const u8;
    let mut len = 0;
    while len < 1024 && unsafe { *path_ptr.add(len) } != 0 { len += 1; }
    let avail = unsafe { *bufsize } as usize;
    if len + 1 > avail {
        unsafe { *bufsize = (len + 1) as u32 };
        return -1;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(path_ptr, buf as *mut u8, len + 1);
        *bufsize = len as u32;
    }
    0
}

// Darwin TLV (Thread Local Variables) bootstrap.
// All TLV descriptors in an image share ONE pthread key. The key is stored
// in each descriptor at offset 8. On first call, we create the key and
// allocate a large block. Each TLV variable lives at its own offset within the block.
//
// IMPORTANT: Apple's __tlv_bootstrap is a tiny leaf function that only clobbers rax.
// The compiler relies on this and does NOT save caller-saved registers around the call.
// Our Rust implementation calls libc functions (clobbering rcx/rdx/rsi/r8-r11), so we
// wrap it with an assembly shim that preserves all caller-saved registers except rax.
static TLV_KEY: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static TLV_KEY_INIT: std::sync::Once = std::sync::Once::new();

// TLV init image: __thread_data contains non-zero initial values for thread-locals.
// When allocating a new TLV block, we must copy these initial values (not just calloc).
static mut TLV_INIT_IMAGE: *const u8 = std::ptr::null();
static mut TLV_INIT_SIZE: usize = 0;
static mut TLV_TOTAL_SIZE: usize = 0;

/// Set the TLV initialization image from the binary's __thread_data and __thread_bss sections.
pub fn set_tlv_init_image(image_addr: *const u8, image_size: usize, total_size: usize) {
    unsafe {
        TLV_INIT_IMAGE = image_addr;
        TLV_INIT_SIZE = image_size;
        TLV_TOTAL_SIZE = total_size;
    }
}

std::arch::global_asm!(
    "shim_tlv_bootstrap_asm:",
    "push rcx",
    "push rdx",
    "push rsi",
    "push r8",
    "push r9",
    "push r10",
    "push r11",
    "call {fn}",
    "pop r11",
    "pop r10",
    "pop r9",
    "pop r8",
    "pop rsi",
    "pop rdx",
    "pop rcx",
    "ret",
    fn = sym shim_tlv_bootstrap,
);
unsafe extern "C" {
    fn shim_tlv_bootstrap_asm(descriptor: *mut u64) -> *mut u8;
}

unsafe extern "C" fn shim_tlv_bootstrap(descriptor: *mut u64) -> *mut u8 {
    let offset = unsafe { *descriptor.add(2) } as usize;

    // Must be ALLOW for all libc calls (Once uses futex, pthread uses futex)
    selector_allow();

    TLV_KEY_INIT.call_once(|| {
        let mut key: libc::pthread_key_t = 0;
        unsafe { libc::pthread_key_create(&mut key, Some(libc::free)) };
        TLV_KEY.store(key as u32, Ordering::Release);
    });

    let key = TLV_KEY.load(Ordering::Acquire) as libc::pthread_key_t;
    unsafe { *descriptor.add(1) = key as u64 };

    let mut block = unsafe { libc::pthread_getspecific(key) } as *mut u8;
    if block.is_null() {
        // Allocate TLV block and initialize from __thread_data image.
        // Darwin copies __thread_data (non-zero init) then zeros __thread_bss.
        let total = unsafe { TLV_TOTAL_SIZE };
        let alloc_size = if total > 0 { total } else { 1024 * 1024 };
        block = unsafe { libc::calloc(1, alloc_size) } as *mut u8;
        if !block.is_null() {
            let img = unsafe { TLV_INIT_IMAGE };
            let img_size = unsafe { TLV_INIT_SIZE };
            if !img.is_null() && img_size > 0 {
                unsafe { std::ptr::copy_nonoverlapping(img, block, img_size) };
            }
            unsafe { libc::pthread_setspecific(key, block as *const libc::c_void) };
        }
    }
    selector_block();

    if block.is_null() { return std::ptr::null_mut(); }
    unsafe { block.add(offset) }
}

// Mach stubs for Rust std
// sysctl — translate Darwin MIBs to Linux values
// Go runtime uses: CTL_HW(6)+HW_NCPU(3), CTL_HW(6)+HW_PAGESIZE(7), CTL_HW(6)+HW_MEMSIZE(24)
// readdir — Darwin dirent has different layout from Linux.
// For simplicity, call Linux readdir and translate the result pointer.
// Darwin dirent64: d_ino(u64), d_seekoff(u64), d_reclen(u16), d_namlen(u16), d_type(u8), d_name[1024]
#[repr(C)]
struct DarwinDirent {
    d_ino: u64,
    d_seekoff: u64,
    d_reclen: u16,
    d_namlen: u16,
    d_type: u8,
    d_name: [u8; 1024],
}

static mut DARWIN_DIRENT_BUF: DarwinDirent = DarwinDirent {
    d_ino: 0, d_seekoff: 0, d_reclen: 0, d_namlen: 0, d_type: 0, d_name: [0; 1024],
};

unsafe extern "C" fn shim_readdir(dirp: *mut libc::DIR) -> *mut DarwinDirent {
    selector_allow();
    let linux_ent = unsafe { libc::readdir(dirp) };
    selector_block();
    if linux_ent.is_null() { return std::ptr::null_mut(); }
    let ent = unsafe { &*linux_ent };
    let buf = &raw mut DARWIN_DIRENT_BUF;
    unsafe {
        (*buf).d_ino = ent.d_ino;
        (*buf).d_seekoff = 0;
        (*buf).d_type = ent.d_type;
        let name_ptr = ent.d_name.as_ptr() as *const u8;
        let mut len = 0;
        while len < 1023 && *name_ptr.add(len) != 0 { len += 1; }
        (*buf).d_namlen = len as u16;
        (*buf).d_reclen = (21 + len + 1) as u16; // header + name + null
        std::ptr::copy_nonoverlapping(name_ptr, (*buf).d_name.as_mut_ptr(), len);
        (*buf).d_name[len] = 0;
    }
    buf as *mut DarwinDirent
}

// Mach time — use Linux clock_gettime(CLOCK_MONOTONIC) as nanoseconds.
unsafe extern "C" fn shim_mach_absolute_time() -> u64 {
    let mut ts: libc::timespec = unsafe { std::mem::zeroed() };
    selector_allow();
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    selector_block();
    (ts.tv_sec as u64) * 1_000_000_000 + (ts.tv_nsec as u64)
}

// Darwin mach_timebase_info: on x86_64 macOS, numer=denom=1 (ticks = nanoseconds).
#[repr(C)]
struct MachTimebaseInfo {
    numer: u32,
    denom: u32,
}

unsafe extern "C" fn shim_mach_timebase_info(info: *mut MachTimebaseInfo) -> i32 {
    if !info.is_null() {
        unsafe {
            (*info).numer = 1;
            (*info).denom = 1;
        }
    }
    0 // KERN_SUCCESS
}

unsafe extern "C" fn shim_sysctl(
    name: *const i32, namelen: u32, oldp: *mut u8, oldlenp: *mut usize,
    _newp: *const u8, _newlen: usize,
) -> i32 {
    if name.is_null() || namelen < 2 { return -1; }
    let mib0 = unsafe { *name };
    let mib1 = unsafe { *name.add(1) };

    // CTL_HW = 6
    if mib0 == 6 {
        match mib1 {
            3 => { // HW_NCPU
                if !oldp.is_null() && !oldlenp.is_null() {
                    selector_allow();
                    let ncpu = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) } as i32;
                    selector_block();
                    let val = if ncpu > 0 { ncpu } else { 1 };
                    let len = unsafe { *oldlenp };
                    if len >= 4 { unsafe { *(oldp as *mut i32) = val; *oldlenp = 4; } }
                }
                return 0;
            }
            7 => { // HW_PAGESIZE
                if !oldp.is_null() && !oldlenp.is_null() {
                    let len = unsafe { *oldlenp };
                    if len >= 4 { unsafe { *(oldp as *mut i32) = 4096; *oldlenp = 4; } }
                }
                return 0;
            }
            24 => { // HW_MEMSIZE (u64)
                if !oldp.is_null() && !oldlenp.is_null() {
                    let len = unsafe { *oldlenp };
                    if len >= 8 { unsafe { *(oldp as *mut u64) = 8 * 1024 * 1024 * 1024; *oldlenp = 8; } } // 8GB fake
                }
                return 0;
            }
            _ => {}
        }
    }
    // CTL_KERN = 1
    if mib0 == 1 {
        match mib1 {
            14 => { // KERN_MAXPROC — just return something
                if !oldp.is_null() && !oldlenp.is_null() {
                    let len = unsafe { *oldlenp };
                    if len >= 4 { unsafe { *(oldp as *mut i32) = 2048; *oldlenp = 4; } }
                }
                return 0;
            }
            _ => {}
        }
    }
    // Unknown MIB — return success with no data (Go handles this)
    0
}

unsafe extern "C" fn shim_sysctlbyname(
    name: *const i8, oldp: *mut u8, oldlenp: *mut usize,
    _newp: *const u8, _newlen: usize,
) -> i32 {
    if name.is_null() { return -1; }
    let name_str = unsafe { std::ffi::CStr::from_ptr(name) }.to_str().unwrap_or("");
    match name_str {
        "hw.ncpu" | "hw.logicalcpu" | "hw.physicalcpu" => {
            if !oldp.is_null() && !oldlenp.is_null() {
                selector_allow();
                let ncpu = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) } as i32;
                selector_block();
                let val = if ncpu > 0 { ncpu } else { 1 };
                let len = unsafe { *oldlenp };
                if len >= 4 { unsafe { *(oldp as *mut i32) = val; *oldlenp = 4; } }
            }
            0
        }
        "hw.pagesize" => {
            if !oldp.is_null() && !oldlenp.is_null() {
                let len = unsafe { *oldlenp };
                if len >= 4 { unsafe { *(oldp as *mut i32) = 4096; *oldlenp = 4; } }
            }
            0
        }
        "hw.memsize" => {
            if !oldp.is_null() && !oldlenp.is_null() {
                let len = unsafe { *oldlenp };
                if len >= 8 { unsafe { *(oldp as *mut u64) = 8 * 1024 * 1024 * 1024; *oldlenp = 8; } }
            }
            0
        }
        _ => 0 // unknown — return success
    }
}

unsafe extern "C" fn shim_mach_task_self() -> u32 {
    crate::mach_ipc::task_self_port()
}
unsafe extern "C" fn shim_mach_thread_self() -> u32 {
    crate::mach_ipc::thread_self_port()
}
unsafe extern "C" fn shim_host_self() -> u32 {
    crate::mach_ipc::host_self_port()
}
unsafe extern "C" fn shim_mach_reply_port() -> u32 {
    crate::mach_ipc::reply_port()
}

static mut BOOTSTRAP_PORT_VAL: u32 = crate::mach_ipc::SPECIAL_PORT_BOOTSTRAP;
unsafe extern "C" fn shim_bootstrap_port_addr() -> *mut u32 {
    unsafe { &raw mut BOOTSTRAP_PORT_VAL }
}

unsafe extern "C" fn shim_mach_port_allocate(task: u32, right: u32, name: *mut u32) -> i32 {
    let _ = task;
    match crate::mach_ipc::port_allocate(right) {
        Ok(n) => {
            if !name.is_null() { unsafe { *name = n } };
            crate::mach_ipc::KERN_SUCCESS
        }
        Err(e) => e,
    }
}

unsafe extern "C" fn shim_mach_port_deallocate(task: u32, name: u32) -> i32 {
    let _ = task;
    crate::mach_ipc::port_deallocate(name)
}

unsafe extern "C" fn shim_mach_port_insert_right(task: u32, name: u32, poly: u32, poly_type: u32) -> i32 {
    let _ = task;
    crate::mach_ipc::port_insert_right(name, poly, poly_type)
}

unsafe extern "C" fn shim_mach_port_mod_refs(task: u32, name: u32, right: u32, delta: i32) -> i32 {
    let _ = task;
    crate::mach_ipc::port_mod_refs(name, right, delta)
}

unsafe extern "C" fn shim_mach_port_type(task: u32, name: u32, ptype: *mut u32) -> i32 {
    let _ = task;
    let (ret, t) = crate::mach_ipc::port_type(name);
    if !ptype.is_null() { unsafe { *ptype = t } };
    ret
}

unsafe extern "C" fn shim_mach_msg(
    msg: *mut crate::mach_ipc::MachMsgHeader,
    option: i32,
    send_size: u32,
    rcv_size: u32,
    rcv_name: u32,
    timeout: u32,
    notify: u32,
) -> i32 {
    log::trace!("mach_msg: option={:#x} send={} rcv={} rcv_name={:#x}",
        option, send_size, rcv_size, rcv_name);
    unsafe { crate::mach_ipc::mach_msg(msg, option, send_size, rcv_size, rcv_name, timeout, notify) }
}

// mach_vm_protect(task, addr, size, set_max, prot) → kern_return_t
// Translate Darwin VM_PROT to Linux PROT and call mprotect
unsafe extern "C" fn shim_mach_vm_protect(
    _task: u32, addr: u64, size: u64, _set_max: i32, prot: i32,
) -> i32 {
    // Darwin VM_PROT: VM_PROT_READ=1, VM_PROT_WRITE=2, VM_PROT_EXECUTE=4 — same as Linux PROT_*
    let aligned_addr = addr & !0xFFF;
    let aligned_size = ((addr + size + 0xFFF) & !0xFFF) - aligned_addr;
    selector_allow();
    let ret = unsafe { libc::mprotect(aligned_addr as *mut _, aligned_size as usize, prot) };
    selector_block();
    if ret == 0 { 0 } else { 1 } // KERN_SUCCESS=0, KERN_FAILURE=1
}

// vm_protect — older Mach API, same args but 32-bit size
unsafe extern "C" fn shim_vm_protect(
    _task: u32, addr: u64, size: u32, _set_max: i32, prot: i32,
) -> i32 {
    unsafe { shim_mach_vm_protect(_task, addr, size as u64, _set_max, prot) }
}

// Darwin pthread_get_stack{addr,size}_np — return stack bounds.
// Darwin: stackaddr = TOP of stack (highest address), size = total size.
// Rust computes guard page as: stackaddr - stacksize + guard_size.
// We must ensure this yields a valid address BELOW our current rsp.
static STACK_BASE_VAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static STACK_SIZE_VAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(8 * 1024 * 1024);

pub fn set_stack_bounds(base: u64, size: u64) {
    STACK_BASE_VAL.store(base, Ordering::Release);
    STACK_SIZE_VAL.store(size, Ordering::Release);
}

unsafe extern "C" fn shim_pthread_get_stackaddr_np(_thread: u64) -> *mut libc::c_void {
    let base = STACK_BASE_VAL.load(Ordering::Acquire);
    let size = STACK_SIZE_VAL.load(Ordering::Acquire);
    let result = if base != 0 {
        (base + size) as *mut libc::c_void
    } else {
        let mut rsp: u64;
        unsafe { std::arch::asm!("mov {}, rsp", out(reg) rsp) };
        ((rsp + 0x800000) & !0xFFF) as *mut libc::c_void
    };
    result
}

unsafe extern "C" fn shim_pthread_get_stacksize_np(_thread: u64) -> libc::size_t {
    STACK_SIZE_VAL.load(Ordering::Acquire) as libc::size_t
}

// pthread_setname_np on Darwin takes (const char*) for current thread only
// Linux takes (pthread_t, const char*). Bridge by ignoring thread arg.
unsafe extern "C" fn shim_pthread_setname_np(name: *const i8) -> i32 {
    selector_allow();
    let me = unsafe { libc::pthread_self() };
    let ret = unsafe { libc::pthread_setname_np(me, name) };
    selector_block();
    ret
}

// GCD dispatch_semaphore — minimal implementation using POSIX semaphores
// Darwin dispatch_semaphore_t is an opaque pointer. We use a box'd sem_t.
unsafe extern "C" fn shim_dispatch_semaphore_create(value: i64) -> *mut libc::sem_t {
    let sem = Box::into_raw(Box::new(unsafe { std::mem::zeroed::<libc::sem_t>() }));
    selector_allow();
    unsafe { libc::sem_init(sem, 0, value as u32) };
    selector_block();
    sem
}
unsafe extern "C" fn shim_dispatch_semaphore_signal(sem: *mut libc::sem_t) -> i64 {
    if sem.is_null() { return 0; }
    selector_allow();
    let ret = unsafe { libc::sem_post(sem) };
    selector_block();
    ret as i64
}
unsafe extern "C" fn shim_dispatch_semaphore_wait(sem: *mut libc::sem_t, timeout: u64) -> i64 {
    if sem.is_null() { return -1; }
    let _ = timeout; // TODO: handle timeout properly
    selector_allow();
    let ret = unsafe { libc::sem_wait(sem) };
    selector_block();
    ret as i64
}
// dispatch_time(DISPATCH_TIME_NOW, nsec) → absolute time. We return nsec as-is.
unsafe extern "C" fn shim_dispatch_time(when: u64, delta: i64) -> u64 {
    when.wrapping_add(delta as u64)
}
static mut STACK_CHK_GUARD: usize = 0xdeadbeef;
static mut NXARGC: i32 = 0;
static mut NXARGV: *const *const i8 = std::ptr::null();

