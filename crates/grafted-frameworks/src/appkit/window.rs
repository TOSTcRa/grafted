//! NSWindow — wraps WindowServer bridge windows with ObjC interface.
//!
//! NSWindow manages a CGContext for drawing and an X11 window via the
//! WindowServer bridge. Content is drawn by the window's contentView.

use crate::cg::geometry::*;
use crate::cg::context;
use crate::ws::display;

/// NSWindowStyleMask
pub const NS_WINDOW_STYLE_MASK_BORDERLESS: u64 = 0;
pub const NS_WINDOW_STYLE_MASK_TITLED: u64 = 1 << 0;
pub const NS_WINDOW_STYLE_MASK_CLOSABLE: u64 = 1 << 1;
pub const NS_WINDOW_STYLE_MASK_MINIATURIZABLE: u64 = 1 << 2;
pub const NS_WINDOW_STYLE_MASK_RESIZABLE: u64 = 1 << 3;

/// NSBackingStoreType
pub const NS_BACKING_STORE_BUFFERED: u64 = 2;

/// Internal window data stored at the ObjC instance.
#[repr(C)]
pub struct NSWindowData {
    pub window_id: display::WindowID,
    pub frame: CGRect,
    pub style_mask: u64,
    pub title: [u8; 256],
    pub title_len: usize,
    pub context: context::CGContextRef,
    pub content_view: *mut u8, // NSView*
}

/// Get NSWindowData from an ObjC instance.
/// By convention, our NSWindow instances store NSWindowData starting at offset 16.
const WINDOW_DATA_OFFSET: usize = 16;

unsafe fn window_data(obj: *mut u8) -> *mut NSWindowData {
    obj.add(WINDOW_DATA_OFFSET) as *mut NSWindowData
}

/// ObjC method: -[NSWindow initWithContentRect:styleMask:backing:defer:]
pub unsafe extern "C" fn ns_window_init(
    self_: *mut u8,
    _sel: *mut u8,
    content_rect: CGRect,
    style_mask: u64,
    _backing: u64,
    _defer: bool,
) -> *mut u8 {
    let data = unsafe { &mut *window_data(self_) };
    data.frame = content_rect;
    data.style_mask = style_mask;
    data.content_view = std::ptr::null_mut();

    // Create the backing CGContext
    let w = content_rect.size.width as usize;
    let h = content_rect.size.height as usize;
    data.context = unsafe {
        context::CGBitmapContextCreate(
            std::ptr::null_mut(), w, h, 8, w * 4,
            std::ptr::null(), 0,
        )
    };

    // Create the X11 window via WindowServer bridge
    let title = "Untitled";
    data.title[..title.len()].copy_from_slice(title.as_bytes());
    data.title_len = title.len();

    if let Some(id) = display::create_window(
        content_rect.origin.x as i32,
        content_rect.origin.y as i32,
        w as u32,
        h as u32,
        title,
    ) {
        data.window_id = id;
    }

    self_
}

/// ObjC method: -[NSWindow setTitle:]
pub unsafe extern "C" fn ns_window_set_title(
    self_: *mut u8,
    _sel: *mut u8,
    title: *mut u8, // NSString* / CFStringRef
) {
    if title.is_null() { return; }
    // Read CFString bytes
    let cf = title as *const crate::cf::string::CFStringInner;
    let data = unsafe { &mut *window_data(self_) };
    let bytes = unsafe { &(*cf).bytes };
    let len = bytes.len().min(255);
    data.title[..len].copy_from_slice(&bytes[..len]);
    data.title_len = len;
    // TODO: update X11 window title
}

/// ObjC method: -[NSWindow makeKeyAndOrderFront:]
pub unsafe extern "C" fn ns_window_make_key_and_order_front(
    self_: *mut u8,
    _sel: *mut u8,
    _sender: *mut u8,
) {
    let data = unsafe { &*window_data(self_) };
    display::show_window(data.window_id);
}

/// ObjC method: -[NSWindow orderOut:]
pub unsafe extern "C" fn ns_window_order_out(
    self_: *mut u8,
    _sel: *mut u8,
    _sender: *mut u8,
) {
    let data = unsafe { &*window_data(self_) };
    display::hide_window(data.window_id);
}

/// ObjC method: -[NSWindow close]
pub unsafe extern "C" fn ns_window_close(
    self_: *mut u8,
    _sel: *mut u8,
) {
    let data = unsafe { &*window_data(self_) };
    display::destroy_window(data.window_id);
}

/// ObjC method: -[NSWindow display]
/// Flushes the backing CGContext to the X11 window.
pub unsafe extern "C" fn ns_window_display(
    self_: *mut u8,
    _sel: *mut u8,
) {
    let data = unsafe { &*window_data(self_) };
    if !data.context.is_null() {
        display::flush_window(data.window_id, data.context);
    }
}

/// ObjC method: -[NSWindow frame]
pub unsafe extern "C" fn ns_window_frame(
    self_: *mut u8,
    _sel: *mut u8,
) -> CGRect {
    let data = unsafe { &*window_data(self_) };
    data.frame
}

/// ObjC method: -[NSWindow setContentView:]
pub unsafe extern "C" fn ns_window_set_content_view(
    self_: *mut u8,
    _sel: *mut u8,
    view: *mut u8,
) {
    let data = unsafe { &mut *window_data(self_) };
    data.content_view = view;
}

/// ObjC method: -[NSWindow contentView]
pub unsafe extern "C" fn ns_window_content_view(
    self_: *mut u8,
    _sel: *mut u8,
) -> *mut u8 {
    let data = unsafe { &*window_data(self_) };
    data.content_view
}

/// ObjC method: -[NSWindow graphicsContext] — returns the CGContext
pub unsafe extern "C" fn ns_window_graphics_context(
    self_: *mut u8,
    _sel: *mut u8,
) -> context::CGContextRef {
    let data = unsafe { &*window_data(self_) };
    data.context
}
