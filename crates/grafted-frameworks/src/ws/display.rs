//! Display server - manages windows and bridges CGContext to X11.

use std::collections::HashMap;
use std::sync::Mutex;

use super::x11;
use crate::cg::context::CGContextRef;
use crate::cg::geometry::*;

/// Window identifier (CGWindowID on Darwin).
pub type WindowID = u32;

/// A managed window with its backing store and X11 window.
struct ManagedWindow {
    x11_window: x11::Window,
    x11_gc: x11::GC,
    title: String,
    frame: CGRect,
    visible: bool,
}

/// Global display state.
struct DisplayState {
    display: *mut x11::Display,
    screen: i32,
    root: x11::Window,
    visual: *mut x11::Visual,
    depth: i32,
    wm_delete: x11::Atom,
    windows: HashMap<WindowID, ManagedWindow>,
    next_id: WindowID,
    x11_to_id: HashMap<x11::Window, WindowID>, // reverse map
}

unsafe impl Send for DisplayState {}

static DISPLAY: Mutex<Option<DisplayState>> = Mutex::new(None);

/// Initialize the display connection. Returns false if X11 is unavailable.
pub fn init_display() -> bool {
    let Some(lib) = x11::x11() else { return false };

    let mut state = DISPLAY.lock().unwrap();
    if state.is_some() { return true; } // already init

    let dpy = unsafe { (lib.open_display)(std::ptr::null()) };
    if dpy.is_null() {
        log::warn!("WindowServer: cannot open X11 display");
        return false;
    }

    let screen = unsafe { (lib.default_screen)(dpy) };
    let root = unsafe { (lib.root_window)(dpy, screen) };
    let visual = unsafe { (lib.default_visual)(dpy, screen) };
    let depth = unsafe { (lib.default_depth)(dpy, screen) };

    let wm_delete = unsafe {
        (lib.intern_atom)(dpy, b"WM_DELETE_WINDOW\0".as_ptr() as *const i8, 0)
    };

    log::info!("WindowServer: X11 display opened (screen={}, depth={})", screen, depth);

    *state = Some(DisplayState {
        display: dpy,
        screen, root, visual, depth, wm_delete,
        windows: HashMap::new(),
        next_id: 1,
        x11_to_id: HashMap::new(),
    });
    true
}

/// Get the X11 connection file descriptor (for epoll integration with CFRunLoop).
pub fn display_fd() -> Option<i32> {
    let lib = x11::x11()?;
    let state = DISPLAY.lock().unwrap();
    let ds = state.as_ref()?;
    Some(unsafe { (lib.connection_number)(ds.display) })
}

/// Create a new window. Returns the window ID.
pub fn create_window(x: i32, y: i32, width: u32, height: u32, title: &str) -> Option<WindowID> {
    let lib = x11::x11()?;
    let mut state = DISPLAY.lock().unwrap();
    let ds = state.as_mut()?;

    let xwin = unsafe {
        (lib.create_simple_window)(
            ds.display, ds.root,
            x, y, width, height,
            1,   // border width
            0,   // border color (black)
            0xFFFFFF, // background (white)
        )
    };

    let gc = unsafe { (lib.create_gc)(ds.display, xwin, 0, std::ptr::null()) };

    // Set window title
    let c_title = std::ffi::CString::new(title).unwrap_or_default();
    unsafe { (lib.store_name)(ds.display, xwin, c_title.as_ptr()) };

    // Register for WM_DELETE_WINDOW
    let mut protocols = [ds.wm_delete];
    unsafe { (lib.set_wm_protocols)(ds.display, xwin, protocols.as_mut_ptr(), 1) };

    // Select events
    unsafe { (lib.select_input)(ds.display, xwin, x11::ALL_EVENTS_MASK) };

    let id = ds.next_id;
    ds.next_id += 1;

    ds.x11_to_id.insert(xwin, id);
    ds.windows.insert(id, ManagedWindow {
        x11_window: xwin,
        x11_gc: gc,
        title: title.to_string(),
        frame: CGRect {
            origin: CGPoint { x: x as f64, y: y as f64 },
            size: CGSize { width: width as f64, height: height as f64 },
        },
        visible: false,
    });

    log::info!("WindowServer: created window {} ({}x{}) '{}'", id, width, height, title);
    Some(id)
}

/// Show a window.
pub fn show_window(id: WindowID) -> bool {
    let lib = match x11::x11() { Some(l) => l, None => return false };
    let mut state = DISPLAY.lock().unwrap();
    let ds = match state.as_mut() { Some(s) => s, None => return false };
    let win = match ds.windows.get_mut(&id) { Some(w) => w, None => return false };

    unsafe { (lib.map_window)(ds.display, win.x11_window) };
    unsafe { (lib.flush)(ds.display) };
    win.visible = true;
    true
}

/// Hide a window.
pub fn hide_window(id: WindowID) -> bool {
    let lib = match x11::x11() { Some(l) => l, None => return false };
    let mut state = DISPLAY.lock().unwrap();
    let ds = match state.as_mut() { Some(s) => s, None => return false };
    let win = match ds.windows.get_mut(&id) { Some(w) => w, None => return false };

    unsafe { (lib.unmap_window)(ds.display, win.x11_window) };
    unsafe { (lib.flush)(ds.display) };
    win.visible = false;
    true
}

/// Destroy a window.
pub fn destroy_window(id: WindowID) -> bool {
    let lib = match x11::x11() { Some(l) => l, None => return false };
    let mut state = DISPLAY.lock().unwrap();
    let ds = match state.as_mut() { Some(s) => s, None => return false };

    if let Some(win) = ds.windows.remove(&id) {
        ds.x11_to_id.remove(&win.x11_window);
        unsafe { (lib.free_gc)(ds.display, win.x11_gc) };
        unsafe { (lib.destroy_window)(ds.display, win.x11_window) };
        unsafe { (lib.flush)(ds.display) };
        true
    } else {
        false
    }
}

/// Blit a CGContext's pixel buffer to the window.
pub fn flush_window(id: WindowID, ctx: CGContextRef) -> bool {
    if ctx.is_null() { return false; }
    let lib = match x11::x11() { Some(l) => l, None => return false };
    let mut state = DISPLAY.lock().unwrap();
    let ds = match state.as_mut() { Some(s) => s, None => return false };
    let win = match ds.windows.get(&id) { Some(w) => w, None => return false };

    let c = unsafe { &mut *ctx };
    let width = c.width as u32;
    let height = c.height as u32;

    // Create XImage from our BGRA pixel buffer.
    // Allocate via libc::malloc so XDestroyImage can free() it correctly.
    let pixel_bytes = (width * height * 4) as usize;
    let data = unsafe { libc::malloc(pixel_bytes) } as *mut u8;
    if data.is_null() { return false; }
    unsafe { std::ptr::copy_nonoverlapping(c.pixels.as_ptr(), data, pixel_bytes) };

    let ximage = unsafe {
        (lib.create_image)(
            ds.display,
            ds.visual,
            ds.depth as u32,
            x11::Z_PIXMAP,
            0,                      // offset
            data,
            width, height,
            32,                     // bitmap_pad
            0,                      // bytes_per_line (auto)
        )
    };

    if ximage.is_null() {
        unsafe { libc::free(data as *mut _) };
        return false;
    }

    unsafe {
        (lib.put_image)(
            ds.display, win.x11_window, win.x11_gc,
            ximage,
            0, 0, // src x, y
            0, 0, // dst x, y
            width, height,
        );
        // XDestroyImage frees the data pointer (allocated via libc::malloc above)
        (lib.destroy_image)(ximage);
        (lib.flush)(ds.display);
    }

    true
}

/// Representation of a display event routed to the application.
#[derive(Debug, Clone)]
pub enum DisplayEvent {
    Expose { window: WindowID },
    KeyDown { window: WindowID, keycode: u32 },
    KeyUp { window: WindowID, keycode: u32 },
    MouseDown { window: WindowID, x: i32, y: i32, button: u32 },
    MouseUp { window: WindowID, x: i32, y: i32, button: u32 },
    MouseMove { window: WindowID, x: i32, y: i32 },
    WindowClose { window: WindowID },
    Resize { window: WindowID, width: u32, height: u32 },
}

/// Poll for pending events. Returns a list of translated events.
pub fn poll_events() -> Vec<DisplayEvent> {
    let lib = match x11::x11() { Some(l) => l, None => return Vec::new() };
    let state = DISPLAY.lock().unwrap();
    let ds = match state.as_ref() { Some(s) => s, None => return Vec::new() };

    let mut events = Vec::new();
    while unsafe { (lib.pending)(ds.display) } > 0 {
        let mut xev: x11::XEvent = [0u8; 192];
        unsafe { (lib.next_event)(ds.display, &mut xev) };

        // Event type is the first i32
        let ev_type = i32::from_ne_bytes([xev[0], xev[1], xev[2], xev[3]]);
        // Window is typically at offset 32 (XAnyEvent.window)
        let xwin = u64::from_ne_bytes([
            xev[32], xev[33], xev[34], xev[35],
            xev[36], xev[37], xev[38], xev[39],
        ]);

        let wid = match ds.x11_to_id.get(&xwin) {
            Some(&id) => id,
            None => continue,
        };

        match ev_type {
            x11::EXPOSE => events.push(DisplayEvent::Expose { window: wid }),
            x11::KEY_PRESS => {
                let keycode = u32::from_ne_bytes([xev[84], xev[85], xev[86], xev[87]]);
                events.push(DisplayEvent::KeyDown { window: wid, keycode });
            }
            x11::KEY_RELEASE => {
                let keycode = u32::from_ne_bytes([xev[84], xev[85], xev[86], xev[87]]);
                events.push(DisplayEvent::KeyUp { window: wid, keycode });
            }
            x11::BUTTON_PRESS => {
                let x = i32::from_ne_bytes([xev[64], xev[65], xev[66], xev[67]]);
                let y = i32::from_ne_bytes([xev[68], xev[69], xev[70], xev[71]]);
                let button = u32::from_ne_bytes([xev[84], xev[85], xev[86], xev[87]]);
                events.push(DisplayEvent::MouseDown { window: wid, x, y, button });
            }
            x11::BUTTON_RELEASE => {
                let x = i32::from_ne_bytes([xev[64], xev[65], xev[66], xev[67]]);
                let y = i32::from_ne_bytes([xev[68], xev[69], xev[70], xev[71]]);
                let button = u32::from_ne_bytes([xev[84], xev[85], xev[86], xev[87]]);
                events.push(DisplayEvent::MouseUp { window: wid, x, y, button });
            }
            x11::MOTION_NOTIFY => {
                let x = i32::from_ne_bytes([xev[64], xev[65], xev[66], xev[67]]);
                let y = i32::from_ne_bytes([xev[68], xev[69], xev[70], xev[71]]);
                events.push(DisplayEvent::MouseMove { window: wid, x, y });
            }
            x11::CONFIGURE_NOTIFY => {
                let w = u32::from_ne_bytes([xev[56], xev[57], xev[58], xev[59]]);
                let h = u32::from_ne_bytes([xev[60], xev[61], xev[62], xev[63]]);
                events.push(DisplayEvent::Resize { window: wid, width: w, height: h });
            }
            x11::CLIENT_MESSAGE => {
                // Check for WM_DELETE_WINDOW
                let data32 = u64::from_ne_bytes([
                    xev[56], xev[57], xev[58], xev[59],
                    xev[60], xev[61], xev[62], xev[63],
                ]);
                if data32 == ds.wm_delete {
                    events.push(DisplayEvent::WindowClose { window: wid });
                }
            }
            _ => {}
        }
    }
    events
}
