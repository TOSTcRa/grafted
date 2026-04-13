//! Raw X11 bindings via dlopen — no compile-time libX11 dependency.
//!
//! Functions are loaded at runtime from libX11.so.6. If X11 is not available
//! (headless server, Wayland-only), all operations gracefully return None.

use std::ffi::CString;
use std::sync::OnceLock;

// Opaque X11 types
pub type Display = core::ffi::c_void;
pub type Window = u64;
pub type Pixmap = u64;
pub type GC = *mut core::ffi::c_void;
pub type Visual = core::ffi::c_void;
pub type XImage = core::ffi::c_void;
pub type Atom = u64;
pub type KeySym = u64;
pub type XEvent = [u8; 192]; // XEvent union is 192 bytes on x86_64

// X11 constants
pub const EXPOSE: i32 = 12;
pub const KEY_PRESS: i32 = 2;
pub const KEY_RELEASE: i32 = 3;
pub const BUTTON_PRESS: i32 = 4;
pub const BUTTON_RELEASE: i32 = 5;
pub const MOTION_NOTIFY: i32 = 6;
pub const CONFIGURE_NOTIFY: i32 = 22;
pub const CLIENT_MESSAGE: i32 = 33;
pub const DESTROY_NOTIFY: i32 = 17;

pub const EXPOSURE_MASK: i64 = 1 << 15;
pub const KEY_PRESS_MASK: i64 = 1 << 0;
pub const KEY_RELEASE_MASK: i64 = 1 << 1;
pub const BUTTON_PRESS_MASK: i64 = 1 << 2;
pub const BUTTON_RELEASE_MASK: i64 = 1 << 3;
pub const POINTER_MOTION_MASK: i64 = 1 << 6;
pub const STRUCTURE_NOTIFY_MASK: i64 = 1 << 17;

pub const Z_PIXMAP: i32 = 2;

pub const ALL_EVENTS_MASK: i64 = EXPOSURE_MASK | KEY_PRESS_MASK | KEY_RELEASE_MASK
    | BUTTON_PRESS_MASK | BUTTON_RELEASE_MASK | POINTER_MOTION_MASK | STRUCTURE_NOTIFY_MASK;

/// Dynamically loaded X11 function table.
pub struct X11Lib {
    _handle: *mut core::ffi::c_void,
    pub open_display: unsafe extern "C" fn(*const i8) -> *mut Display,
    pub close_display: unsafe extern "C" fn(*mut Display) -> i32,
    pub default_screen: unsafe extern "C" fn(*mut Display) -> i32,
    pub root_window: unsafe extern "C" fn(*mut Display, i32) -> Window,
    pub default_visual: unsafe extern "C" fn(*mut Display, i32) -> *mut Visual,
    pub default_depth: unsafe extern "C" fn(*mut Display, i32) -> i32,
    pub create_simple_window: unsafe extern "C" fn(
        *mut Display, Window, i32, i32, u32, u32, u32, u64, u64,
    ) -> Window,
    pub destroy_window: unsafe extern "C" fn(*mut Display, Window),
    pub map_window: unsafe extern "C" fn(*mut Display, Window),
    pub unmap_window: unsafe extern "C" fn(*mut Display, Window),
    pub select_input: unsafe extern "C" fn(*mut Display, Window, i64),
    pub create_gc: unsafe extern "C" fn(*mut Display, Window, u64, *const core::ffi::c_void) -> GC,
    pub free_gc: unsafe extern "C" fn(*mut Display, GC),
    pub create_image: unsafe extern "C" fn(
        *mut Display, *mut Visual, u32, i32, i32, *mut u8, u32, u32, i32, i32,
    ) -> *mut XImage,
    pub put_image: unsafe extern "C" fn(
        *mut Display, Window, GC, *mut XImage, i32, i32, i32, i32, u32, u32,
    ),
    pub destroy_image: unsafe extern "C" fn(*mut XImage),
    pub flush: unsafe extern "C" fn(*mut Display),
    pub pending: unsafe extern "C" fn(*mut Display) -> i32,
    pub next_event: unsafe extern "C" fn(*mut Display, *mut XEvent),
    pub connection_number: unsafe extern "C" fn(*mut Display) -> i32,
    pub store_name: unsafe extern "C" fn(*mut Display, Window, *const i8),
    pub intern_atom: unsafe extern "C" fn(*mut Display, *const i8, i32) -> Atom,
    pub set_wm_protocols: unsafe extern "C" fn(*mut Display, Window, *mut Atom, i32) -> i32,
}

unsafe impl Send for X11Lib {}
unsafe impl Sync for X11Lib {}

static X11: OnceLock<Option<X11Lib>> = OnceLock::new();

macro_rules! load_sym {
    ($handle:expr, $name:expr, $ty:ty) => {{
        let sym = unsafe { libc::dlsym($handle, concat!($name, "\0").as_ptr() as *const i8) };
        if sym.is_null() {
            log::warn!("X11: missing symbol {}", $name);
            return None;
        }
        unsafe { std::mem::transmute::<*mut core::ffi::c_void, $ty>(sym) }
    }};
}

impl X11Lib {
    fn load() -> Option<Self> {
        let lib_name = CString::new("libX11.so.6").ok()?;
        let handle = unsafe { libc::dlopen(lib_name.as_ptr(), libc::RTLD_LAZY) };
        if handle.is_null() {
            log::info!("X11: libX11.so.6 not found — running headless");
            return None;
        }

        type OpenDisplay = unsafe extern "C" fn(*const i8) -> *mut Display;
        type CloseDisplay = unsafe extern "C" fn(*mut Display) -> i32;
        type IntFn = unsafe extern "C" fn(*mut Display) -> i32;
        type RootWindowFn = unsafe extern "C" fn(*mut Display, i32) -> Window;
        type DefVisualFn = unsafe extern "C" fn(*mut Display, i32) -> *mut Visual;
        type DefDepthFn = unsafe extern "C" fn(*mut Display, i32) -> i32;
        type CreateWindowFn = unsafe extern "C" fn(*mut Display, Window, i32, i32, u32, u32, u32, u64, u64) -> Window;
        type DestroyWindowFn = unsafe extern "C" fn(*mut Display, Window);
        type MapWindowFn = unsafe extern "C" fn(*mut Display, Window);
        type SelectInputFn = unsafe extern "C" fn(*mut Display, Window, i64);
        type CreateGCFn = unsafe extern "C" fn(*mut Display, Window, u64, *const core::ffi::c_void) -> GC;
        type FreeGCFn = unsafe extern "C" fn(*mut Display, GC);
        type CreateImageFn = unsafe extern "C" fn(*mut Display, *mut Visual, u32, i32, i32, *mut u8, u32, u32, i32, i32) -> *mut XImage;
        type PutImageFn = unsafe extern "C" fn(*mut Display, Window, GC, *mut XImage, i32, i32, i32, i32, u32, u32);
        type DestroyImageFn = unsafe extern "C" fn(*mut XImage);
        type FlushFn = unsafe extern "C" fn(*mut Display);
        type NextEventFn = unsafe extern "C" fn(*mut Display, *mut XEvent);
        type ConnNumFn = unsafe extern "C" fn(*mut Display) -> i32;
        type StoreNameFn = unsafe extern "C" fn(*mut Display, Window, *const i8);
        type InternAtomFn = unsafe extern "C" fn(*mut Display, *const i8, i32) -> Atom;
        type SetProtocolsFn = unsafe extern "C" fn(*mut Display, Window, *mut Atom, i32) -> i32;

        Some(Self {
            _handle: handle,
            open_display: load_sym!(handle, "XOpenDisplay", OpenDisplay),
            close_display: load_sym!(handle, "XCloseDisplay", CloseDisplay),
            default_screen: load_sym!(handle, "XDefaultScreen", IntFn),
            root_window: load_sym!(handle, "XRootWindow", RootWindowFn),
            default_visual: load_sym!(handle, "XDefaultVisual", DefVisualFn),
            default_depth: load_sym!(handle, "XDefaultDepth", DefDepthFn),
            create_simple_window: load_sym!(handle, "XCreateSimpleWindow", CreateWindowFn),
            destroy_window: load_sym!(handle, "XDestroyWindow", DestroyWindowFn),
            map_window: load_sym!(handle, "XMapWindow", MapWindowFn),
            unmap_window: load_sym!(handle, "XUnmapWindow", MapWindowFn),
            select_input: load_sym!(handle, "XSelectInput", SelectInputFn),
            create_gc: load_sym!(handle, "XCreateGC", CreateGCFn),
            free_gc: load_sym!(handle, "XFreeGC", FreeGCFn),
            create_image: load_sym!(handle, "XCreateImage", CreateImageFn),
            put_image: load_sym!(handle, "XPutImage", PutImageFn),
            destroy_image: load_sym!(handle, "XDestroyImage", DestroyImageFn),
            flush: load_sym!(handle, "XFlush", FlushFn),
            pending: load_sym!(handle, "XPending", IntFn),
            next_event: load_sym!(handle, "XNextEvent", NextEventFn),
            connection_number: load_sym!(handle, "XConnectionNumber", ConnNumFn),
            store_name: load_sym!(handle, "XStoreName", StoreNameFn),
            intern_atom: load_sym!(handle, "XInternAtom", InternAtomFn),
            set_wm_protocols: load_sym!(handle, "XSetWMProtocols", SetProtocolsFn),
        })
    }
}

/// Get the global X11 library handle. Returns None if X11 is unavailable.
pub fn x11() -> Option<&'static X11Lib> {
    X11.get_or_init(X11Lib::load).as_ref()
}
