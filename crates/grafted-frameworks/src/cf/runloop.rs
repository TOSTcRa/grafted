//! CFRunLoop — event loop backed by epoll on Linux.
//!
//! Every GUI app runs a CFRunLoop on its main thread. The run loop waits for
//! events (timers, sources, Mach ports) and dispatches them. On Darwin this
//! uses kqueue/mach_msg; we use epoll + timerfd.

use super::types::*;
use super::string::CFStringRef;

pub type CFRunLoopRef = *mut CFRunLoopInner;
pub type CFRunLoopSourceRef = *mut CFRunLoopSourceInner;
pub type CFRunLoopTimerRef = *mut CFRunLoopTimerInner;
pub type CFRunLoopMode = *const core::ffi::c_void;

// Well-known run loop modes (constant string pointers)
// Run loop mode strings (C-compatible pointers)
#[unsafe(no_mangle)]
pub static kCFRunLoopDefaultMode: [u8; 22] = *b"kCFRunLoopDefaultMode\0";
#[unsafe(no_mangle)]
pub static kCFRunLoopCommonModes: [u8; 22] = *b"kCFRunLoopCommonModes\0";

// CFRunLoopSource callback context (version 0)
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CFRunLoopSourceContext {
    pub version: CFIndex,
    pub info: *mut core::ffi::c_void,
    pub retain: Option<unsafe extern "C" fn(*const core::ffi::c_void) -> *const core::ffi::c_void>,
    pub release: Option<unsafe extern "C" fn(*const core::ffi::c_void)>,
    pub copy_description: Option<unsafe extern "C" fn(*const core::ffi::c_void) -> CFStringRef>,
    pub equal: Option<unsafe extern "C" fn(*const core::ffi::c_void, *const core::ffi::c_void) -> CFBoolean>,
    pub hash: Option<unsafe extern "C" fn(*const core::ffi::c_void) -> CFHashCode>,
    pub schedule: Option<unsafe extern "C" fn(*const core::ffi::c_void, CFRunLoopRef, CFRunLoopMode)>,
    pub cancel: Option<unsafe extern "C" fn(*const core::ffi::c_void, CFRunLoopRef, CFRunLoopMode)>,
    pub perform: Option<unsafe extern "C" fn(*const core::ffi::c_void)>,
}

// CFRunLoopTimer callback
pub type CFRunLoopTimerCallBack = Option<unsafe extern "C" fn(CFRunLoopTimerRef, *mut core::ffi::c_void)>;

pub type CFAbsoluteTime = f64;
pub type CFTimeInterval = f64;

// Run loop activity flags
pub const K_CF_RUNLOOP_ENTRY: u64 = 1 << 0;
pub const K_CF_RUNLOOP_BEFORE_TIMERS: u64 = 1 << 1;
pub const K_CF_RUNLOOP_BEFORE_SOURCES: u64 = 1 << 2;
pub const K_CF_RUNLOOP_BEFORE_WAITING: u64 = 1 << 5;
pub const K_CF_RUNLOOP_AFTER_WAITING: u64 = 1 << 6;
pub const K_CF_RUNLOOP_EXIT: u64 = 1 << 7;

// Run result
pub const K_CF_RUNLOOP_RUN_FINISHED: i32 = 1;
pub const K_CF_RUNLOOP_RUN_STOPPED: i32 = 2;
pub const K_CF_RUNLOOP_RUN_TIMED_OUT: i32 = 3;
pub const K_CF_RUNLOOP_RUN_HANDLED_SOURCE: i32 = 4;

pub struct CFRunLoopInner {
    pub base: CFRuntimeBase,
    pub sources: Vec<CFRunLoopSourceRef>,
    pub timers: Vec<CFRunLoopTimerRef>,
    pub stopped: bool,
    pub epoll_fd: i32,
}

pub struct CFRunLoopSourceInner {
    pub base: CFRuntimeBase,
    pub context: CFRunLoopSourceContext,
    pub signaled: bool,
}

pub struct CFRunLoopTimerInner {
    pub base: CFRuntimeBase,
    pub fire_date: CFAbsoluteTime,
    pub interval: CFTimeInterval,
    pub callback: CFRunLoopTimerCallBack,
    pub info: *mut core::ffi::c_void,
}

// Global: one run loop per thread
thread_local! {
    static CURRENT_RUNLOOP: std::cell::Cell<CFRunLoopRef> = const { std::cell::Cell::new(std::ptr::null_mut()) };
}

fn get_or_create_runloop() -> CFRunLoopRef {
    CURRENT_RUNLOOP.with(|c| {
        let rl = c.get();
        if !rl.is_null() { return rl; }
        let epfd = unsafe { libc::epoll_create1(0) };
        let new_rl = Box::into_raw(Box::new(CFRunLoopInner {
            base: CFRuntimeBase::new(CF_RUNLOOP_TYPE_ID),
            sources: Vec::new(),
            timers: Vec::new(),
            stopped: false,
            epoll_fd: epfd,
        }));
        c.set(new_rl);
        new_rl
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopGetCurrent() -> CFRunLoopRef {
    get_or_create_runloop()
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopGetMain() -> CFRunLoopRef {
    // For now, main = current (single main thread assumption)
    get_or_create_runloop()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFRunLoopRun() {
    let _rl = get_or_create_runloop();
    loop {
        let result = unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode.as_ptr() as CFRunLoopMode, 1e10, 0) };
        if result == K_CF_RUNLOOP_RUN_STOPPED || result == K_CF_RUNLOOP_RUN_FINISHED {
            break;
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFRunLoopRunInMode(
    _mode: CFRunLoopMode,
    seconds: CFTimeInterval,
    return_after_source: CFBoolean,
) -> i32 {
    let rl = get_or_create_runloop();
    let inner = unsafe { &mut *rl };

    if inner.stopped {
        inner.stopped = false;
        return K_CF_RUNLOOP_RUN_STOPPED;
    }

    // Check signaled sources
    for src_ptr in &inner.sources {
        let src = unsafe { &mut **src_ptr };
        if src.signaled {
            src.signaled = false;
            if let Some(perform) = src.context.perform {
                unsafe { perform(src.context.info) };
            }
            if return_after_source != 0 {
                return K_CF_RUNLOOP_RUN_HANDLED_SOURCE;
            }
        }
    }

    // Check timers
    let now = current_absolute_time();
    for timer_ptr in &inner.timers {
        let timer = unsafe { &mut **timer_ptr };
        if now >= timer.fire_date {
            if let Some(cb) = timer.callback {
                unsafe { cb(*timer_ptr, timer.info) };
            }
            if timer.interval > 0.0 {
                timer.fire_date = now + timer.interval;
            } else {
                timer.fire_date = f64::MAX;
            }
            if return_after_source != 0 {
                return K_CF_RUNLOOP_RUN_HANDLED_SOURCE;
            }
        }
    }

    // Wait on epoll
    let timeout_ms = if seconds > 1e9 { -1 } else { (seconds * 1000.0) as i32 };
    let mut ev: libc::epoll_event = unsafe { std::mem::zeroed() };
    let _n = unsafe { libc::epoll_wait(inner.epoll_fd, &mut ev, 1, timeout_ms.min(100)) };

    if inner.stopped {
        inner.stopped = false;
        return K_CF_RUNLOOP_RUN_STOPPED;
    }

    K_CF_RUNLOOP_RUN_TIMED_OUT
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFRunLoopStop(rl: CFRunLoopRef) {
    if rl.is_null() { return; }
    unsafe { (*rl).stopped = true };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFRunLoopWakeUp(_rl: CFRunLoopRef) {
    // TODO: write to an eventfd to wake the epoll
}

// ---- Sources ----

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFRunLoopSourceCreate(
    _alloc: CFAllocatorRef,
    _order: CFIndex,
    context: *const CFRunLoopSourceContext,
) -> CFRunLoopSourceRef {
    let ctx = if context.is_null() {
        unsafe { std::mem::zeroed() }
    } else {
        unsafe { *context }
    };
    Box::into_raw(Box::new(CFRunLoopSourceInner {
        base: CFRuntimeBase::new(CF_RUNLOOP_SOURCE_TYPE_ID),
        context: ctx,
        signaled: false,
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFRunLoopSourceSignal(source: CFRunLoopSourceRef) {
    if !source.is_null() {
        unsafe { (*source).signaled = true };
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFRunLoopAddSource(
    rl: CFRunLoopRef,
    source: CFRunLoopSourceRef,
    _mode: CFRunLoopMode,
) {
    if rl.is_null() || source.is_null() { return; }
    unsafe { (*rl).sources.push(source) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFRunLoopRemoveSource(
    rl: CFRunLoopRef,
    source: CFRunLoopSourceRef,
    _mode: CFRunLoopMode,
) {
    if rl.is_null() || source.is_null() { return; }
    unsafe { (*rl).sources.retain(|s| *s != source) };
}

// ---- Timers ----

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFRunLoopTimerCreate(
    _alloc: CFAllocatorRef,
    fire_date: CFAbsoluteTime,
    interval: CFTimeInterval,
    _flags: CFOptionFlags,
    _order: CFIndex,
    callback: CFRunLoopTimerCallBack,
    _context: *const core::ffi::c_void,
) -> CFRunLoopTimerRef {
    Box::into_raw(Box::new(CFRunLoopTimerInner {
        base: CFRuntimeBase::new(CF_RUNLOOP_TIMER_TYPE_ID),
        fire_date,
        interval,
        callback,
        info: std::ptr::null_mut(),
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFRunLoopAddTimer(
    rl: CFRunLoopRef,
    timer: CFRunLoopTimerRef,
    _mode: CFRunLoopMode,
) {
    if rl.is_null() || timer.is_null() { return; }
    unsafe { (*rl).timers.push(timer) };
}

// ---- Time ----

#[unsafe(no_mangle)]
pub extern "C" fn CFAbsoluteTimeGetCurrent() -> CFAbsoluteTime {
    current_absolute_time()
}

fn current_absolute_time() -> f64 {
    // CF absolute time: seconds since Jan 1 2001 00:00:00 UTC
    // Unix epoch to CF epoch: 978307200 seconds
    let mut ts: libc::timespec = unsafe { std::mem::zeroed() };
    unsafe { libc::clock_gettime(libc::CLOCK_REALTIME, &mut ts) };
    (ts.tv_sec as f64 + ts.tv_nsec as f64 / 1e9) - 978307200.0
}
