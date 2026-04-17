//! CFRunLoop - event loop with per-mode source/timer/observer registries.

use super::types::*;
use super::string::CFStringRef;
use std::collections::{HashMap, HashSet};

pub type CFRunLoopRef = *mut CFRunLoopInner;
pub type CFRunLoopSourceRef = *mut CFRunLoopSourceInner;
pub type CFRunLoopTimerRef = *mut CFRunLoopTimerInner;
pub type CFRunLoopObserverRef = *mut CFRunLoopObserverInner;
pub type CFRunLoopMode = *const core::ffi::c_void;

// Well-known run loop mode name bytes. Callers pass `.as_ptr()` as the mode
// argument; we read bytes at that pointer and match by value.
#[unsafe(no_mangle)]
pub static kCFRunLoopDefaultMode: [u8; 22] = *b"kCFRunLoopDefaultMode\0";
#[unsafe(no_mangle)]
pub static kCFRunLoopCommonModes: [u8; 22] = *b"kCFRunLoopCommonModes\0";
#[unsafe(no_mangle)]
pub static NSEventTrackingRunLoopMode: [u8; 27] = *b"NSEventTrackingRunLoopMode\0";
#[unsafe(no_mangle)]
pub static NSModalPanelRunLoopMode: [u8; 24] = *b"NSModalPanelRunLoopMode\0";
#[unsafe(no_mangle)]
pub static UITrackingRunLoopMode: [u8; 22] = *b"UITrackingRunLoopMode\0";

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

// CFRunLoopObserver callback: (observer, activity_flags, info)
pub type CFRunLoopObserverCallBack = Option<unsafe extern "C" fn(CFRunLoopObserverRef, u64, *mut core::ffi::c_void)>;

pub type CFAbsoluteTime = f64;
pub type CFTimeInterval = f64;

// Run loop activity flags (bitfield, also used as observer "activities" mask)
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
    /// Per-mode registries. Mode name ("kCFRunLoopDefaultMode", "NSEventTrackingRunLoopMode", ...) -> contents.
    pub modes: HashMap<String, CFRunLoopModeInner>,
    /// Set of mode names that count as "common". Items added to kCFRunLoopCommonModes
    /// are registered in every common mode at the time of registration.
    pub common_modes: HashSet<String>,
    /// Stack of currently-entered modes. Top of stack is `currentMode`. A nested
    /// `CFRunLoopRunInMode` call pushes, the return pops.
    pub mode_stack: Vec<String>,
    pub stopped: bool,
    pub epoll_fd: i32,
}

pub struct CFRunLoopModeInner {
    pub sources: Vec<CFRunLoopSourceRef>,
    pub timers: Vec<CFRunLoopTimerRef>,
    pub observers: Vec<CFRunLoopObserverRef>,
}

impl CFRunLoopModeInner {
    fn new() -> Self {
        Self { sources: Vec::new(), timers: Vec::new(), observers: Vec::new() }
    }
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

pub struct CFRunLoopObserverInner {
    pub base: CFRuntimeBase,
    pub activities: u64,
    pub repeats: bool,
    pub order: CFIndex,
    pub callback: CFRunLoopObserverCallBack,
    pub info: *mut core::ffi::c_void,
}

// Global: one run loop per thread
thread_local! {
    static CURRENT_RUNLOOP: std::cell::Cell<CFRunLoopRef> = const { std::cell::Cell::new(std::ptr::null_mut()) };
}

/// Resolve a mode pointer to its string name. Accepts our static-byte-array
fn mode_name_from_ptr(mode: CFRunLoopMode) -> String {
    if mode.is_null() {
        return "kCFRunLoopDefaultMode".to_string();
    }
    // Try to read up to 64 bytes at the pointer, stop at NUL. Accept only
    // printable ASCII (safety against reading garbage as a mode name).
    let bytes = unsafe {
        let p = mode as *const u8;
        let mut n = 0usize;
        while n < 64 {
            let b = std::ptr::read_volatile(p.add(n));
            if b == 0 { break; }
            if !(b.is_ascii_graphic() || b == b' ') {
                return "kCFRunLoopDefaultMode".to_string();
            }
            n += 1;
        }
        std::slice::from_raw_parts(p, n)
    };
    match std::str::from_utf8(bytes) {
        Ok(s) if !s.is_empty() => s.to_string(),
        _ => "kCFRunLoopDefaultMode".to_string(),
    }
}

fn get_or_create_runloop() -> CFRunLoopRef {
    CURRENT_RUNLOOP.with(|c| {
        let rl = c.get();
        if !rl.is_null() { return rl; }
        let epfd = unsafe { libc::epoll_create1(0) };
        let mut modes = HashMap::new();
        modes.insert("kCFRunLoopDefaultMode".to_string(), CFRunLoopModeInner::new());
        modes.insert("NSEventTrackingRunLoopMode".to_string(), CFRunLoopModeInner::new());
        modes.insert("NSModalPanelRunLoopMode".to_string(), CFRunLoopModeInner::new());
        let mut common_modes = HashSet::new();
        common_modes.insert("kCFRunLoopDefaultMode".to_string());
        common_modes.insert("NSEventTrackingRunLoopMode".to_string());
        let new_rl = Box::into_raw(Box::new(CFRunLoopInner {
            base: CFRuntimeBase::new(CF_RUNLOOP_TYPE_ID),
            modes,
            common_modes,
            mode_stack: Vec::new(),
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
    // Single main thread assumption
    get_or_create_runloop()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFRunLoopCopyCurrentMode(rl: CFRunLoopRef) -> CFStringRef {
    if rl.is_null() { return std::ptr::null_mut(); }
    let rl = unsafe { &*rl };
    // We return a raw byte pointer to the name. Callers that expect a real
    match rl.mode_stack.last().map(String::as_str) {
        Some("kCFRunLoopDefaultMode") | None => kCFRunLoopDefaultMode.as_ptr() as CFStringRef,
        Some("NSEventTrackingRunLoopMode") => NSEventTrackingRunLoopMode.as_ptr() as CFStringRef,
        Some("NSModalPanelRunLoopMode") => NSModalPanelRunLoopMode.as_ptr() as CFStringRef,
        Some("UITrackingRunLoopMode") => UITrackingRunLoopMode.as_ptr() as CFStringRef,
        _ => kCFRunLoopDefaultMode.as_ptr() as CFStringRef,
    }
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

/// Fire observers registered in `mode` whose activity mask overlaps `activity`.
unsafe fn fire_observers(rl: &mut CFRunLoopInner, mode: &str, activity: u64) {
    let to_fire: Vec<CFRunLoopObserverRef> = rl.modes.get(mode)
        .map(|m| m.observers.clone())
        .unwrap_or_default();
    for obs in to_fire {
        let o = unsafe { &*obs };
        if o.activities & activity != 0 {
            if let Some(cb) = o.callback {
                unsafe { cb(obs, activity, o.info) };
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFRunLoopRunInMode(
    mode: CFRunLoopMode,
    seconds: CFTimeInterval,
    return_after_source: CFBoolean,
) -> i32 {
    let rl_ptr = get_or_create_runloop();
    let mode_name = mode_name_from_ptr(mode);

    // Mode must exist in the registry (auto-create on first use so user modes work)
    {
        let rl = unsafe { &mut *rl_ptr };
        rl.modes.entry(mode_name.clone()).or_insert_with(CFRunLoopModeInner::new);
        if rl.stopped {
            rl.stopped = false;
            return K_CF_RUNLOOP_RUN_STOPPED;
        }
        rl.mode_stack.push(mode_name.clone());
    }

    unsafe {
        let rl = &mut *rl_ptr;
        fire_observers(rl, &mode_name, K_CF_RUNLOOP_ENTRY);
    }

    // Active modes = ONLY the requested mode. `kCFRunLoopCommonModes` is a
    let active_modes: Vec<String> = vec![mode_name.clone()];

    // === Source dispatch (one pass) ===
    let sources_to_check: Vec<CFRunLoopSourceRef> = {
        let rl = unsafe { &*rl_ptr };
        active_modes.iter()
            .filter_map(|m| rl.modes.get(m))
            .flat_map(|m| m.sources.iter().copied())
            .collect()
    };
    unsafe { fire_observers(&mut *rl_ptr, &mode_name, K_CF_RUNLOOP_BEFORE_SOURCES); }
    for src_ptr in sources_to_check {
        let src = unsafe { &mut *src_ptr };
        if src.signaled {
            src.signaled = false;
            if let Some(perform) = src.context.perform {
                unsafe { perform(src.context.info) };
            }
            if return_after_source != 0 {
                unsafe {
                    let rl = &mut *rl_ptr;
                    fire_observers(rl, &mode_name, K_CF_RUNLOOP_EXIT);
                    rl.mode_stack.pop();
                }
                return K_CF_RUNLOOP_RUN_HANDLED_SOURCE;
            }
        }
    }

    // === Timer dispatch ===
    let now = current_absolute_time();
    let timers_to_check: Vec<CFRunLoopTimerRef> = {
        let rl = unsafe { &*rl_ptr };
        active_modes.iter()
            .filter_map(|m| rl.modes.get(m))
            .flat_map(|m| m.timers.iter().copied())
            .collect()
    };
    unsafe { fire_observers(&mut *rl_ptr, &mode_name, K_CF_RUNLOOP_BEFORE_TIMERS); }
    for timer_ptr in timers_to_check {
        let timer = unsafe { &mut *timer_ptr };
        if now >= timer.fire_date {
            if let Some(cb) = timer.callback {
                unsafe { cb(timer_ptr, timer.info) };
            }
            if timer.interval > 0.0 {
                timer.fire_date = now + timer.interval;
            } else {
                timer.fire_date = f64::MAX;
            }
            if return_after_source != 0 {
                unsafe {
                    let rl = &mut *rl_ptr;
                    fire_observers(rl, &mode_name, K_CF_RUNLOOP_EXIT);
                    rl.mode_stack.pop();
                }
                return K_CF_RUNLOOP_RUN_HANDLED_SOURCE;
            }
        }
    }

    // === Wait (epoll) ===
    unsafe { fire_observers(&mut *rl_ptr, &mode_name, K_CF_RUNLOOP_BEFORE_WAITING); }
    let timeout_ms = if seconds > 1e9 { -1 } else { (seconds * 1000.0) as i32 };
    let mut ev: libc::epoll_event = unsafe { std::mem::zeroed() };
    let epfd = unsafe { (*rl_ptr).epoll_fd };
    let _n = unsafe { libc::epoll_wait(epfd, &mut ev, 1, timeout_ms.min(100)) };
    unsafe { fire_observers(&mut *rl_ptr, &mode_name, K_CF_RUNLOOP_AFTER_WAITING); }

    let result = {
        let rl = unsafe { &mut *rl_ptr };
        if rl.stopped {
            rl.stopped = false;
            K_CF_RUNLOOP_RUN_STOPPED
        } else {
            K_CF_RUNLOOP_RUN_TIMED_OUT
        }
    };

    unsafe {
        let rl = &mut *rl_ptr;
        fire_observers(rl, &mode_name, K_CF_RUNLOOP_EXIT);
        rl.mode_stack.pop();
    }
    result
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
pub unsafe extern "C" fn CFRunLoopContainsSource(
    rl: CFRunLoopRef,
    source: CFRunLoopSourceRef,
    mode: CFRunLoopMode,
) -> CFBoolean {
    if rl.is_null() || source.is_null() { return 0; }
    let mode_name = mode_name_from_ptr(mode);
    let rl = unsafe { &*rl };
    if mode_name == "kCFRunLoopCommonModes" {
        for m in rl.common_modes.iter() {
            if let Some(reg) = rl.modes.get(m) {
                if reg.sources.contains(&source) { return 1; }
            }
        }
        return 0;
    }
    rl.modes.get(&mode_name)
        .map(|m| m.sources.contains(&source) as CFBoolean)
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFRunLoopAddSource(
    rl: CFRunLoopRef,
    source: CFRunLoopSourceRef,
    mode: CFRunLoopMode,
) {
    if rl.is_null() || source.is_null() { return; }
    let mode_name = mode_name_from_ptr(mode);
    let rl = unsafe { &mut *rl };
    if mode_name == "kCFRunLoopCommonModes" {
        // Register in all current common modes
        let commons: Vec<String> = rl.common_modes.iter().cloned().collect();
        for m in commons {
            let entry = rl.modes.entry(m).or_insert_with(CFRunLoopModeInner::new);
            if !entry.sources.contains(&source) {
                entry.sources.push(source);
            }
        }
    } else {
        let entry = rl.modes.entry(mode_name).or_insert_with(CFRunLoopModeInner::new);
        if !entry.sources.contains(&source) {
            entry.sources.push(source);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFRunLoopRemoveSource(
    rl: CFRunLoopRef,
    source: CFRunLoopSourceRef,
    mode: CFRunLoopMode,
) {
    if rl.is_null() || source.is_null() { return; }
    let mode_name = mode_name_from_ptr(mode);
    let rl = unsafe { &mut *rl };
    if mode_name == "kCFRunLoopCommonModes" {
        let commons: Vec<String> = rl.common_modes.iter().cloned().collect();
        for m in commons {
            if let Some(entry) = rl.modes.get_mut(&m) {
                entry.sources.retain(|&s| s != source);
            }
        }
    } else if let Some(entry) = rl.modes.get_mut(&mode_name) {
        entry.sources.retain(|&s| s != source);
    }
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
    mode: CFRunLoopMode,
) {
    if rl.is_null() || timer.is_null() { return; }
    let mode_name = mode_name_from_ptr(mode);
    let rl = unsafe { &mut *rl };
    if mode_name == "kCFRunLoopCommonModes" {
        let commons: Vec<String> = rl.common_modes.iter().cloned().collect();
        for m in commons {
            let entry = rl.modes.entry(m).or_insert_with(CFRunLoopModeInner::new);
            if !entry.timers.contains(&timer) {
                entry.timers.push(timer);
            }
        }
    } else {
        let entry = rl.modes.entry(mode_name).or_insert_with(CFRunLoopModeInner::new);
        if !entry.timers.contains(&timer) {
            entry.timers.push(timer);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFRunLoopRemoveTimer(
    rl: CFRunLoopRef,
    timer: CFRunLoopTimerRef,
    mode: CFRunLoopMode,
) {
    if rl.is_null() || timer.is_null() { return; }
    let mode_name = mode_name_from_ptr(mode);
    let rl = unsafe { &mut *rl };
    if mode_name == "kCFRunLoopCommonModes" {
        let commons: Vec<String> = rl.common_modes.iter().cloned().collect();
        for m in commons {
            if let Some(entry) = rl.modes.get_mut(&m) {
                entry.timers.retain(|&t| t != timer);
            }
        }
    } else if let Some(entry) = rl.modes.get_mut(&mode_name) {
        entry.timers.retain(|&t| t != timer);
    }
}

// ---- Observers ----

#[repr(C)]
pub struct CFRunLoopObserverContext {
    pub version: CFIndex,
    pub info: *mut core::ffi::c_void,
    pub retain: Option<unsafe extern "C" fn(*const core::ffi::c_void) -> *const core::ffi::c_void>,
    pub release: Option<unsafe extern "C" fn(*const core::ffi::c_void)>,
    pub copy_description: Option<unsafe extern "C" fn(*const core::ffi::c_void) -> CFStringRef>,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFRunLoopObserverCreate(
    _alloc: CFAllocatorRef,
    activities: u64,
    repeats: CFBoolean,
    order: CFIndex,
    callback: CFRunLoopObserverCallBack,
    context: *const CFRunLoopObserverContext,
) -> CFRunLoopObserverRef {
    let info = if context.is_null() {
        std::ptr::null_mut()
    } else {
        unsafe { (*context).info }
    };
    Box::into_raw(Box::new(CFRunLoopObserverInner {
        base: CFRuntimeBase::new(CF_RUNLOOP_TYPE_ID), // reuse type id; observers don't need their own
        activities,
        repeats: repeats != 0,
        order,
        callback,
        info,
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFRunLoopAddObserver(
    rl: CFRunLoopRef,
    observer: CFRunLoopObserverRef,
    mode: CFRunLoopMode,
) {
    if rl.is_null() || observer.is_null() { return; }
    let mode_name = mode_name_from_ptr(mode);
    let rl = unsafe { &mut *rl };
    if mode_name == "kCFRunLoopCommonModes" {
        let commons: Vec<String> = rl.common_modes.iter().cloned().collect();
        for m in commons {
            let entry = rl.modes.entry(m).or_insert_with(CFRunLoopModeInner::new);
            if !entry.observers.contains(&observer) {
                entry.observers.push(observer);
            }
        }
    } else {
        let entry = rl.modes.entry(mode_name).or_insert_with(CFRunLoopModeInner::new);
        if !entry.observers.contains(&observer) {
            entry.observers.push(observer);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFRunLoopRemoveObserver(
    rl: CFRunLoopRef,
    observer: CFRunLoopObserverRef,
    mode: CFRunLoopMode,
) {
    if rl.is_null() || observer.is_null() { return; }
    let mode_name = mode_name_from_ptr(mode);
    let rl = unsafe { &mut *rl };
    if mode_name == "kCFRunLoopCommonModes" {
        let commons: Vec<String> = rl.common_modes.iter().cloned().collect();
        for m in commons {
            if let Some(entry) = rl.modes.get_mut(&m) {
                entry.observers.retain(|&o| o != observer);
            }
        }
    } else if let Some(entry) = rl.modes.get_mut(&mode_name) {
        entry.observers.retain(|&o| o != observer);
    }
}

// ---- Common modes ----

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFRunLoopAddCommonMode(
    rl: CFRunLoopRef,
    mode: CFRunLoopMode,
) {
    if rl.is_null() { return; }
    let mode_name = mode_name_from_ptr(mode);
    let rl = unsafe { &mut *rl };
    rl.common_modes.insert(mode_name.clone());
    rl.modes.entry(mode_name).or_insert_with(CFRunLoopModeInner::new);
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


#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static TEST_PERFORM_COUNT: AtomicU32 = AtomicU32::new(0);

    unsafe extern "C" fn test_perform(_info: *const core::ffi::c_void) {
        TEST_PERFORM_COUNT.fetch_add(1, Ordering::SeqCst);
    }

    fn fresh_runloop() -> CFRunLoopRef {
        CURRENT_RUNLOOP.with(|c| c.set(std::ptr::null_mut()));
        get_or_create_runloop()
    }

    fn make_source() -> CFRunLoopSourceRef {
        let ctx = CFRunLoopSourceContext {
            version: 0,
            info: std::ptr::null_mut(),
            retain: None, release: None, copy_description: None,
            equal: None, hash: None, schedule: None, cancel: None,
            perform: Some(test_perform),
        };
        unsafe { CFRunLoopSourceCreate(std::ptr::null_mut(), 0, &ctx) }
    }

    #[test]
    fn source_in_default_mode_does_not_fire_in_tracking_mode() {
        TEST_PERFORM_COUNT.store(0, Ordering::SeqCst);
        let rl = fresh_runloop();
        let src = make_source();
        unsafe {
            CFRunLoopAddSource(rl, src, kCFRunLoopDefaultMode.as_ptr() as CFRunLoopMode);
            CFRunLoopSourceSignal(src);
            // Run in NSEventTrackingRunLoopMode - source is registered only in default
            CFRunLoopRunInMode(
                NSEventTrackingRunLoopMode.as_ptr() as CFRunLoopMode,
                0.0, 1,
            );
        }
        assert_eq!(TEST_PERFORM_COUNT.load(Ordering::SeqCst), 0,
            "source registered in default mode should NOT fire when running in tracking mode");
    }

    #[test]
    fn source_in_common_modes_fires_in_tracking_mode() {
        TEST_PERFORM_COUNT.store(0, Ordering::SeqCst);
        let rl = fresh_runloop();
        let src = make_source();
        unsafe {
            CFRunLoopAddSource(rl, src, kCFRunLoopCommonModes.as_ptr() as CFRunLoopMode);
            CFRunLoopSourceSignal(src);
            CFRunLoopRunInMode(
                NSEventTrackingRunLoopMode.as_ptr() as CFRunLoopMode,
                0.0, 1,
            );
        }
        assert_eq!(TEST_PERFORM_COUNT.load(Ordering::SeqCst), 1,
            "source registered in common modes SHOULD fire when running in tracking mode");
    }

    #[test]
    fn source_in_tracking_mode_only_fires_there() {
        TEST_PERFORM_COUNT.store(0, Ordering::SeqCst);
        let rl = fresh_runloop();
        let src = make_source();
        unsafe {
            CFRunLoopAddSource(rl, src, NSEventTrackingRunLoopMode.as_ptr() as CFRunLoopMode);
            CFRunLoopSourceSignal(src);
            // Run in default mode - should NOT fire
            CFRunLoopRunInMode(
                kCFRunLoopDefaultMode.as_ptr() as CFRunLoopMode,
                0.0, 1,
            );
        }
        assert_eq!(TEST_PERFORM_COUNT.load(Ordering::SeqCst), 0,
            "source registered only in tracking mode should NOT fire in default mode");

        TEST_PERFORM_COUNT.store(0, Ordering::SeqCst);
        unsafe {
            CFRunLoopSourceSignal(src);
            CFRunLoopRunInMode(
                NSEventTrackingRunLoopMode.as_ptr() as CFRunLoopMode,
                0.0, 1,
            );
        }
        assert_eq!(TEST_PERFORM_COUNT.load(Ordering::SeqCst), 1,
            "source registered in tracking mode SHOULD fire when running in tracking mode");
    }

    // Observer ordering test
    static OBS_LOG: std::sync::Mutex<Vec<u64>> = std::sync::Mutex::new(Vec::new());

    unsafe extern "C" fn test_observer(_obs: CFRunLoopObserverRef, activity: u64, _info: *mut core::ffi::c_void) {
        OBS_LOG.lock().unwrap().push(activity);
    }

    #[test]
    fn observers_fire_in_entry_before_waiting_exit_order() {
        OBS_LOG.lock().unwrap().clear();
        let rl = fresh_runloop();
        let ctx = CFRunLoopObserverContext {
            version: 0, info: std::ptr::null_mut(),
            retain: None, release: None, copy_description: None,
        };
        let obs = unsafe {
            CFRunLoopObserverCreate(
                std::ptr::null_mut(),
                K_CF_RUNLOOP_ENTRY | K_CF_RUNLOOP_BEFORE_WAITING | K_CF_RUNLOOP_EXIT,
                1, 0, Some(test_observer), &ctx,
            )
        };
        unsafe {
            CFRunLoopAddObserver(rl, obs, kCFRunLoopDefaultMode.as_ptr() as CFRunLoopMode);
            CFRunLoopRunInMode(kCFRunLoopDefaultMode.as_ptr() as CFRunLoopMode, 0.0, 0);
        }
        let log = OBS_LOG.lock().unwrap().clone();
        // Expect at least ENTRY first, EXIT last, BEFORE_WAITING in between
        assert!(!log.is_empty(), "observers must have fired at least once");
        assert_eq!(log[0], K_CF_RUNLOOP_ENTRY, "entry observer must fire first");
        assert_eq!(*log.last().unwrap(), K_CF_RUNLOOP_EXIT, "exit observer must fire last");
        assert!(log.contains(&K_CF_RUNLOOP_BEFORE_WAITING),
            "before-waiting observer must fire during wait");
    }

    #[test]
    fn current_mode_reports_entered_mode() {
        let rl = fresh_runloop();
        // No mode entered yet -> default
        unsafe {
            let m = CFRunLoopCopyCurrentMode(rl);
            assert_eq!(m as *const u8, kCFRunLoopDefaultMode.as_ptr());
        }
        // After adding a common mode, it's still default (no mode entered)
        unsafe {
            CFRunLoopAddCommonMode(rl, NSModalPanelRunLoopMode.as_ptr() as CFRunLoopMode);
        }
    }
}
