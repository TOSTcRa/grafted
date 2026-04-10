//! CGContext — 2D drawing context with software pixel buffer backend.
//!
//! This is the core rendering surface. Darwin apps draw into CGContexts
//! which are then composited by WindowServer. We back them with BGRA pixel
//! buffers that can be blitted to X11 XImage or Wayland wl_shm_buffer.

use super::geometry::*;
use super::color::CGColorSpaceRef;
use crate::cf::types::*;

pub type CGContextRef = *mut CGContextInner;

// Bitmap info constants
pub const K_CG_IMAGE_ALPHA_NONE: u32 = 0;
pub const K_CG_IMAGE_ALPHA_PREMULTIPLIED_LAST: u32 = 1;
pub const K_CG_IMAGE_ALPHA_PREMULTIPLIED_FIRST: u32 = 2;
pub const K_CG_BITMAP_BYTE_ORDER_32_LITTLE: u32 = 2 << 12;

pub struct CGContextInner {
    pub base: CFRuntimeBase,
    pub width: usize,
    pub height: usize,
    pub stride: usize, // bytes per row
    pub bits_per_component: usize,
    pub pixels: Vec<u8>, // BGRA or RGBA pixel data
    // Drawing state
    pub fill_color: [f64; 4],   // RGBA
    pub stroke_color: [f64; 4], // RGBA
    pub line_width: CGFloat,
    pub transform: CGAffineTransform,
    // State stack
    pub state_stack: Vec<DrawState>,
}

#[derive(Clone)]
pub struct DrawState {
    pub fill_color: [f64; 4],
    pub stroke_color: [f64; 4],
    pub line_width: CGFloat,
    pub transform: CGAffineTransform,
}

impl CGContextInner {
    fn pixel_offset(&self, x: usize, y: usize) -> usize {
        y * self.stride + x * 4
    }

    fn set_pixel(&mut self, x: usize, y: usize, r: u8, g: u8, b: u8, a: u8) {
        if x >= self.width || y >= self.height { return; }
        let off = self.pixel_offset(x, y);
        if off + 3 < self.pixels.len() {
            // BGRA format (matching X11/Wayland expectations)
            self.pixels[off] = b;
            self.pixels[off + 1] = g;
            self.pixels[off + 2] = r;
            self.pixels[off + 3] = a;
        }
    }
}

// ---- Creation ----

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CGBitmapContextCreate(
    data: *mut u8,
    width: usize,
    height: usize,
    bits_per_component: usize,
    bytes_per_row: usize,
    _color_space: CGColorSpaceRef,
    _bitmap_info: u32,
) -> CGContextRef {
    let stride = if bytes_per_row > 0 { bytes_per_row } else { width * 4 };
    let size = stride * height;

    let pixels = if data.is_null() {
        vec![0u8; size]
    } else {
        unsafe { Vec::from_raw_parts(data, size, size) }
    };

    Box::into_raw(Box::new(CGContextInner {
        base: CFRuntimeBase::new(32),
        width, height, stride, bits_per_component,
        pixels,
        fill_color: [0.0, 0.0, 0.0, 1.0],
        stroke_color: [0.0, 0.0, 0.0, 1.0],
        line_width: 1.0,
        transform: CGAffineTransform::identity(),
        state_stack: Vec::new(),
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CGBitmapContextGetData(ctx: CGContextRef) -> *mut u8 {
    if ctx.is_null() { return std::ptr::null_mut(); }
    unsafe { (*ctx).pixels.as_mut_ptr() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CGBitmapContextGetWidth(ctx: CGContextRef) -> usize {
    if ctx.is_null() { 0 } else { unsafe { (*ctx).width } }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CGBitmapContextGetHeight(ctx: CGContextRef) -> usize {
    if ctx.is_null() { 0 } else { unsafe { (*ctx).height } }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CGBitmapContextGetBytesPerRow(ctx: CGContextRef) -> usize {
    if ctx.is_null() { 0 } else { unsafe { (*ctx).stride } }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CGContextRelease(ctx: CGContextRef) {
    if !ctx.is_null() { let _ = unsafe { Box::from_raw(ctx) }; }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CGContextRetain(ctx: CGContextRef) -> CGContextRef {
    if !ctx.is_null() { unsafe { (*ctx).base.retain() }; }
    ctx
}

// ---- State ----

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CGContextSaveGState(ctx: CGContextRef) {
    if ctx.is_null() { return; }
    let c = unsafe { &mut *ctx };
    c.state_stack.push(DrawState {
        fill_color: c.fill_color,
        stroke_color: c.stroke_color,
        line_width: c.line_width,
        transform: c.transform,
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CGContextRestoreGState(ctx: CGContextRef) {
    if ctx.is_null() { return; }
    let c = unsafe { &mut *ctx };
    if let Some(state) = c.state_stack.pop() {
        c.fill_color = state.fill_color;
        c.stroke_color = state.stroke_color;
        c.line_width = state.line_width;
        c.transform = state.transform;
    }
}

// ---- Colors ----

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CGContextSetRGBFillColor(
    ctx: CGContextRef, r: CGFloat, g: CGFloat, b: CGFloat, a: CGFloat,
) {
    if ctx.is_null() { return; }
    unsafe { (*ctx).fill_color = [r, g, b, a] };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CGContextSetRGBStrokeColor(
    ctx: CGContextRef, r: CGFloat, g: CGFloat, b: CGFloat, a: CGFloat,
) {
    if ctx.is_null() { return; }
    unsafe { (*ctx).stroke_color = [r, g, b, a] };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CGContextSetGrayFillColor(ctx: CGContextRef, gray: CGFloat, a: CGFloat) {
    unsafe { CGContextSetRGBFillColor(ctx, gray, gray, gray, a) };
}

// ---- Drawing ----

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CGContextFillRect(ctx: CGContextRef, rect: CGRect) {
    if ctx.is_null() { return; }
    let c = unsafe { &mut *ctx };
    let [r, g, b, a] = c.fill_color;
    let rb = (r * 255.0) as u8;
    let gb = (g * 255.0) as u8;
    let bb = (b * 255.0) as u8;
    let ab = (a * 255.0) as u8;

    let x0 = rect.origin.x.max(0.0) as usize;
    let y0 = rect.origin.y.max(0.0) as usize;
    let x1 = ((rect.origin.x + rect.size.width) as usize).min(c.width);
    let y1 = ((rect.origin.y + rect.size.height) as usize).min(c.height);

    for y in y0..y1 {
        for x in x0..x1 {
            c.set_pixel(x, y, rb, gb, bb, ab);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CGContextClearRect(ctx: CGContextRef, rect: CGRect) {
    if ctx.is_null() { return; }
    let c = unsafe { &mut *ctx };
    let x0 = rect.origin.x.max(0.0) as usize;
    let y0 = rect.origin.y.max(0.0) as usize;
    let x1 = ((rect.origin.x + rect.size.width) as usize).min(c.width);
    let y1 = ((rect.origin.y + rect.size.height) as usize).min(c.height);
    for y in y0..y1 {
        for x in x0..x1 {
            c.set_pixel(x, y, 0, 0, 0, 0);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CGContextStrokeRect(ctx: CGContextRef, rect: CGRect) {
    if ctx.is_null() { return; }
    let c = unsafe { &mut *ctx };
    let [r, g, b, a] = c.stroke_color;
    let rb = (r * 255.0) as u8;
    let gb = (g * 255.0) as u8;
    let bb = (b * 255.0) as u8;
    let ab = (a * 255.0) as u8;

    let x0 = rect.origin.x.max(0.0) as usize;
    let y0 = rect.origin.y.max(0.0) as usize;
    let x1 = ((rect.origin.x + rect.size.width) as usize).min(c.width.saturating_sub(1));
    let y1 = ((rect.origin.y + rect.size.height) as usize).min(c.height.saturating_sub(1));

    for x in x0..=x1 { c.set_pixel(x, y0, rb, gb, bb, ab); c.set_pixel(x, y1, rb, gb, bb, ab); }
    for y in y0..=y1 { c.set_pixel(x0, y, rb, gb, bb, ab); c.set_pixel(x1, y, rb, gb, bb, ab); }
}

// ---- Transform ----

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CGContextTranslateCTM(ctx: CGContextRef, tx: CGFloat, ty: CGFloat) {
    if ctx.is_null() { return; }
    unsafe { (*ctx).transform.tx += tx; (*ctx).transform.ty += ty; }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CGContextScaleCTM(ctx: CGContextRef, sx: CGFloat, sy: CGFloat) {
    if ctx.is_null() { return; }
    unsafe { (*ctx).transform.a *= sx; (*ctx).transform.d *= sy; }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CGContextGetCTM(ctx: CGContextRef) -> CGAffineTransform {
    if ctx.is_null() { return CGAffineTransform::identity(); }
    unsafe { (*ctx).transform }
}

// ---- Line ----

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CGContextSetLineWidth(ctx: CGContextRef, width: CGFloat) {
    if !ctx.is_null() { unsafe { (*ctx).line_width = width }; }
}
