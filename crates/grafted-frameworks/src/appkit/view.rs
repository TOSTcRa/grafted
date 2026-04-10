//! NSView — the base class for all visual elements.
//!
//! NSView provides a drawing surface (backed by a CGContext) and a
//! coordinate system. Subclasses override drawRect: to render content.

use crate::cg::geometry::*;
use crate::cg::context;

/// Internal view data.
#[repr(C)]
pub struct NSViewData {
    pub frame: CGRect,
    pub bounds: CGRect,
    pub superview: *mut u8,
    pub subviews: [*mut u8; 64], // up to 64 subviews
    pub subview_count: u32,
    pub needs_display: bool,
    pub hidden: bool,
    pub alpha: CGFloat,
    pub background_color: [f64; 4], // RGBA
}

const VIEW_DATA_OFFSET: usize = 16;

unsafe fn view_data(obj: *mut u8) -> *mut NSViewData {
    obj.add(VIEW_DATA_OFFSET) as *mut NSViewData
}

/// ObjC method: -[NSView initWithFrame:]
pub unsafe extern "C" fn ns_view_init_with_frame(
    self_: *mut u8,
    _sel: *mut u8,
    frame: CGRect,
) -> *mut u8 {
    let data = unsafe { &mut *view_data(self_) };
    data.frame = frame;
    data.bounds = CGRect {
        origin: CGPoint { x: 0.0, y: 0.0 },
        size: frame.size,
    };
    data.superview = std::ptr::null_mut();
    data.subviews = [std::ptr::null_mut(); 64];
    data.subview_count = 0;
    data.needs_display = true;
    data.hidden = false;
    data.alpha = 1.0;
    data.background_color = [1.0, 1.0, 1.0, 1.0]; // white
    self_
}

/// ObjC method: -[NSView frame]
pub unsafe extern "C" fn ns_view_frame(
    self_: *mut u8,
    _sel: *mut u8,
) -> CGRect {
    unsafe { (*view_data(self_)).frame }
}

/// ObjC method: -[NSView setFrame:]
pub unsafe extern "C" fn ns_view_set_frame(
    self_: *mut u8,
    _sel: *mut u8,
    frame: CGRect,
) {
    let data = unsafe { &mut *view_data(self_) };
    data.frame = frame;
    data.bounds.size = frame.size;
    data.needs_display = true;
}

/// ObjC method: -[NSView bounds]
pub unsafe extern "C" fn ns_view_bounds(
    self_: *mut u8,
    _sel: *mut u8,
) -> CGRect {
    unsafe { (*view_data(self_)).bounds }
}

/// ObjC method: -[NSView addSubview:]
pub unsafe extern "C" fn ns_view_add_subview(
    self_: *mut u8,
    _sel: *mut u8,
    subview: *mut u8,
) {
    if subview.is_null() { return; }
    let data = unsafe { &mut *view_data(self_) };
    let idx = data.subview_count as usize;
    if idx < 64 {
        data.subviews[idx] = subview;
        data.subview_count += 1;
        let sub_data = unsafe { &mut *view_data(subview) };
        sub_data.superview = self_;
    }
}

/// ObjC method: -[NSView superview]
pub unsafe extern "C" fn ns_view_superview(
    self_: *mut u8,
    _sel: *mut u8,
) -> *mut u8 {
    unsafe { (*view_data(self_)).superview }
}

/// ObjC method: -[NSView setNeedsDisplay:]
pub unsafe extern "C" fn ns_view_set_needs_display(
    self_: *mut u8,
    _sel: *mut u8,
    flag: bool,
) {
    unsafe { (*view_data(self_)).needs_display = flag };
}

/// ObjC method: -[NSView isHidden]
pub unsafe extern "C" fn ns_view_is_hidden(
    self_: *mut u8,
    _sel: *mut u8,
) -> bool {
    unsafe { (*view_data(self_)).hidden }
}

/// ObjC method: -[NSView setHidden:]
pub unsafe extern "C" fn ns_view_set_hidden(
    self_: *mut u8,
    _sel: *mut u8,
    flag: bool,
) {
    unsafe { (*view_data(self_)).hidden = flag };
}

/// ObjC method: -[NSView drawRect:] — base implementation clears with background color
pub unsafe extern "C" fn ns_view_draw_rect(
    _self: *mut u8,
    _sel: *mut u8,
    _rect: CGRect,
) {
    // Base implementation does nothing — subclasses override this
}
