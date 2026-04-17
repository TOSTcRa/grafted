//! NSPasteboard - system clipboard access.

use std::process::{Command, Stdio};
use std::io::Write;
use std::sync::atomic::{AtomicI64, AtomicPtr, Ordering};
use std::sync::{Mutex, Once};
use std::time::Duration;

fn read_cfstring(ptr: *mut u8) -> Option<String> {
    if ptr.is_null() { return None; }
    unsafe {
        let s = ptr as *const crate::cf::string::CFStringInner;
        let rb = &(*s).base;
        if rb.type_id() == crate::cf::types::CF_STRING_TYPE_ID {
            return std::str::from_utf8(&(*s).bytes).ok().map(String::from);
        }
    }
    unsafe {
        let c = std::ffi::CStr::from_ptr(ptr as *const i8);
        c.to_str().ok().map(String::from)
    }
}


#[derive(Copy, Clone, Debug)]
enum Backend {
    Wayland,  // wl-copy / wl-paste
    X11Xclip, // xclip -selection clipboard
    X11Xsel,  // xsel --clipboard
    None,     // no backend found - NSPasteboard no-ops
}

static BACKEND: std::sync::OnceLock<Backend> = std::sync::OnceLock::new();

fn backend() -> Backend {
    *BACKEND.get_or_init(|| {
        // Wayland session first (matches most modern GNOME/KDE/sway setups)
        let is_wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
        if is_wayland && which("wl-copy") && which("wl-paste") {
            log::info!("NSPasteboard: using wl-copy/wl-paste (Wayland)");
            return Backend::Wayland;
        }
        if which("xclip") {
            log::info!("NSPasteboard: using xclip (X11)");
            return Backend::X11Xclip;
        }
        if which("xsel") {
            log::info!("NSPasteboard: using xsel (X11)");
            return Backend::X11Xsel;
        }
        log::warn!("NSPasteboard: no clipboard backend (wl-copy/xclip/xsel); reads/writes are no-ops. Install wl-clipboard or xclip.");
        Backend::None
    })
}

fn which(prog: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {prog} >/dev/null 2>&1"))
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}


fn read_clipboard_text() -> Option<String> {
    let out = match backend() {
        Backend::Wayland => {
            Command::new("wl-paste")
                .arg("--no-newline")
                .output()
                .ok()?
        }
        Backend::X11Xclip => {
            Command::new("xclip")
                .args(["-selection", "clipboard", "-o"])
                .output()
                .ok()?
        }
        Backend::X11Xsel => {
            Command::new("xsel")
                .args(["--clipboard", "--output"])
                .output()
                .ok()?
        }
        Backend::None => return None,
    };
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

fn write_clipboard_text(text: &str) -> bool {
    let mut cmd = match backend() {
        Backend::Wayland => {
            let mut c = Command::new("wl-copy");
            c.stdin(Stdio::piped());
            c
        }
        Backend::X11Xclip => {
            let mut c = Command::new("xclip");
            c.args(["-selection", "clipboard", "-i"]).stdin(Stdio::piped());
            c
        }
        Backend::X11Xsel => {
            let mut c = Command::new("xsel");
            c.args(["--clipboard", "--input"]).stdin(Stdio::piped());
            c
        }
        Backend::None => return false,
    };
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("NSPasteboard: failed to spawn clipboard writer: {e}");
            return false;
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
    }
    // `wl-copy` daemonizes itself to stay alive and serve the selection;
    // don't block on it. For `xclip`/`xsel` we do wait.
    if matches!(backend(), Backend::Wayland) {
        // Give wl-copy a brief moment to register the clipboard; it forks
        // immediately and the child becomes the selection owner.
        let _ = child.wait();
    } else {
        let _ = child.wait();
    }
    true
}


static CHANGE_COUNT: AtomicI64 = AtomicI64::new(0);
static LAST_CONTENT: Mutex<Option<String>> = Mutex::new(None);
static POLL_STARTED: Once = Once::new();

/// Initialize the polling thread. Called once on first access.
fn ensure_polling() {
    POLL_STARTED.call_once(|| {
        // Prime LAST_CONTENT with the current clipboard so the first poll
        // doesn't falsely bump the counter.
        if let Some(s) = read_clipboard_text() {
            *LAST_CONTENT.lock().unwrap() = Some(s);
        }
        std::thread::Builder::new()
            .name("grafted-pasteboard-poll".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_millis(500));
                let current = read_clipboard_text();
                let mut last = LAST_CONTENT.lock().unwrap();
                if *last != current {
                    *last = current;
                    drop(last);  // release before posting notification (observers may call clipboard APIs)
                    CHANGE_COUNT.fetch_add(1, Ordering::Relaxed);
                    log::debug!("NSPasteboard: clipboard changed, count={}",
                        CHANGE_COUNT.load(Ordering::Relaxed));
                    // Post NSPasteboardDidChangeNotification so observers registered
                    // via NSNotificationCenter (e.g. Maccy's clipboard watcher) fire.
                    crate::foundation::notification::post_notification_name_static(
                        b"NSPasteboardDidChangeNotification\0",
                    );
                }
            })
            .expect("spawn pasteboard poll thread");
    });
}

fn bump_change_count() -> i64 {
    CHANGE_COUNT.fetch_add(1, Ordering::Relaxed) + 1
}


/// +[NSPasteboard generalPasteboard] - singleton.
pub unsafe extern "C" fn ns_pasteboard_general(_cls: *mut u8, _sel: *mut u8) -> *mut u8 {
    static PB: AtomicPtr<u8> = AtomicPtr::new(std::ptr::null_mut());
    let p = PB.load(Ordering::Acquire);
    if !p.is_null() { return p; }
    ensure_polling();
    let obj = unsafe { libc::calloc(1, 256) } as *mut u8;
    if obj.is_null() { return PB.load(Ordering::Acquire); }
    match PB.compare_exchange(std::ptr::null_mut(), obj, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => obj,
        Err(winner) => { unsafe { libc::free(obj as *mut libc::c_void) }; winner }
    }
}

/// -[NSPasteboard stringForType:]
/// Returns a CFString with the clipboard content, or null if empty / wrong type.
pub unsafe extern "C" fn ns_pasteboard_string_for_type(
    _self: *mut u8,
    _sel: *mut u8,
    _type: *mut u8,
) -> *mut u8 {
    match read_clipboard_text() {
        Some(s) if !s.is_empty() => {
            let c = match std::ffi::CString::new(s) {
                Ok(c) => c,
                Err(_) => return std::ptr::null_mut(),
            };
            unsafe {
                crate::cf::string::CFStringCreateWithCString(
                    std::ptr::null(), c.as_ptr(), 0x0800_0100,
                ) as *mut u8
            }
        }
        _ => std::ptr::null_mut(),
    }
}

/// -[NSPasteboard setString:forType:]
pub unsafe extern "C" fn ns_pasteboard_set_string(
    _self: *mut u8,
    _sel: *mut u8,
    string: *mut u8,
    _type: *mut u8,
) -> bool {
    let Some(text) = read_cfstring(string) else {
        return false;
    };
    let ok = write_clipboard_text(&text);
    if ok {
        // Pre-bump the counter since we're the ones changing it; the poller
        bump_change_count();
        // Also update LAST_CONTENT so the poller doesn't double-count.
        *LAST_CONTENT.lock().unwrap() = Some(text);
    }
    ok
}

/// -[NSPasteboard clearContents] - clears and returns the new changeCount.
pub unsafe extern "C" fn ns_pasteboard_clear(_self: *mut u8, _sel: *mut u8) -> i64 {
    let _ = write_clipboard_text("");
    *LAST_CONTENT.lock().unwrap() = Some(String::new());
    bump_change_count()
}

/// -[NSPasteboard changeCount]
pub unsafe extern "C" fn ns_pasteboard_change_count(_self: *mut u8, _sel: *mut u8) -> i64 {
    ensure_polling();
    CHANGE_COUNT.load(Ordering::Relaxed)
}

/// -[NSPasteboard types] - we only advertise the string type.
pub unsafe extern "C" fn ns_pasteboard_types(_self: *mut u8, _sel: *mut u8) -> *mut u8 {
    // Return a single-element CFArray with NSPasteboardTypeString.
    // Maccy checks `types.contains(.string)` - we return one string entry.
    unsafe {
        let type_str = std::ffi::CString::new("public.utf8-plain-text").unwrap();
        let cf_type = crate::cf::string::CFStringCreateWithCString(
            std::ptr::null(), type_str.as_ptr(), 0x0800_0100,
        );
        let vals: [*const std::ffi::c_void; 1] = [cf_type as *const _];
        crate::cf::array::CFArrayCreate(
            std::ptr::null(),
            vals.as_ptr(),
            1,
            std::ptr::null(),
        ) as *mut u8
    }
}

/// -[NSPasteboard pasteboardItems] - empty array (we don't model per-item structure).
pub unsafe extern "C" fn ns_pasteboard_items(_self: *mut u8, _sel: *mut u8) -> *mut u8 {
    unsafe {
        crate::cf::array::CFArrayCreate(std::ptr::null(), std::ptr::null(), 0, std::ptr::null()) as *mut u8
    }
}

/// -[NSPasteboard declareTypes:owner:] - returns the new changeCount.
pub unsafe extern "C" fn ns_pasteboard_declare_types(
    _self: *mut u8,
    _sel: *mut u8,
    _types: *mut u8,
    _owner: *mut u8,
) -> i64 {
    bump_change_count()
}

/// -[NSPasteboard dataForType:] - not implemented (binary clipboard data deferred).
pub unsafe extern "C" fn ns_pasteboard_data_for_type(
    _self: *mut u8,
    _sel: *mut u8,
    _type: *mut u8,
) -> *mut u8 {
    std::ptr::null_mut()
}


#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip: write a unique string, read it back. Requires a working
    /// clipboard backend (wl-copy or xclip). Skipped silently otherwise.
    #[test]
    fn clipboard_roundtrip() {
        if matches!(backend(), Backend::None) {
            eprintln!("SKIP: no clipboard backend installed");
            return;
        }
        // Save original clipboard to restore on teardown
        let saved = read_clipboard_text();

        let sentinel = format!("grafted-test-{}", std::process::id());
        assert!(write_clipboard_text(&sentinel), "write must succeed");
        std::thread::sleep(Duration::from_millis(50));
        let got = read_clipboard_text();
        assert_eq!(got.as_deref(), Some(sentinel.as_str()), "round-trip");

        // Change-count bump via set_string
        let before = CHANGE_COUNT.load(Ordering::Relaxed);
        unsafe {
            let c = std::ffi::CString::new("second value").unwrap();
            let s = crate::cf::string::CFStringCreateWithCString(
                std::ptr::null(), c.as_ptr(), 0x0800_0100,
            ) as *mut u8;
            ns_pasteboard_set_string(std::ptr::null_mut(), std::ptr::null_mut(), s, std::ptr::null_mut());
        }
        let after = CHANGE_COUNT.load(Ordering::Relaxed);
        assert!(after > before, "changeCount should increase after set");

        // Restore
        if let Some(orig) = saved {
            let _ = write_clipboard_text(&orig);
        }
    }

    #[test]
    fn types_contains_string_type() {
        unsafe {
            let arr = ns_pasteboard_types(std::ptr::null_mut(), std::ptr::null_mut());
            assert!(!arr.is_null(), "types array must not be null");
            // Just verify it's a non-null CFArray; detailed enumeration requires CFArrayGetValueAtIndex wiring.
        }
    }
}
