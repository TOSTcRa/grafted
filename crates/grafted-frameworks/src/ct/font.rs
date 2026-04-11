//! CTFont + bitmap text rendering.
//!
//! Built-in 8x16 monospace font for basic text display. Each ASCII character
//! (32-126) is stored as 16 bytes, 1 bit per pixel, 8 pixels wide.
//! This lets us render text without external font libraries.

use crate::cf::types::*;
use crate::cg::geometry::*;
use crate::cg::context::CGContextRef;

pub type CTFontRef = *const CTFontInner;

pub struct CTFontInner {
    pub base: CFRuntimeBase,
    pub size: CGFloat,
    pub name: String,
}

// ---- Built-in 8x16 bitmap font (ASCII 32-126) ----
// Each char: 16 bytes, each byte is one row, MSB = leftmost pixel

static BITMAP_FONT: &[u8] = include_bytes!("font_8x16.bin");

/// Get bitmap data for an ASCII character. Returns 16 bytes (one per row).
fn char_bitmap(ch: u8) -> &'static [u8] {
    if ch < 32 || ch > 126 {
        return &BITMAP_FONT[0..16]; // space for unknown chars
    }
    let idx = (ch - 32) as usize * 16;
    if idx + 16 <= BITMAP_FONT.len() {
        &BITMAP_FONT[idx..idx + 16]
    } else {
        &BITMAP_FONT[0..16]
    }
}

// ---- CoreText C API ----

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CTFontCreateWithName(
    _name: CFTypeRef,
    size: CGFloat,
    _matrix: *const core::ffi::c_void,
) -> CTFontRef {
    Box::into_raw(Box::new(CTFontInner {
        base: CFRuntimeBase::new(40),
        size: if size > 0.0 { size } else { 12.0 },
        name: "Grafted-Mono".into(),
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CTFontCreateWithFontDescriptor(
    _descriptor: CFTypeRef,
    size: CGFloat,
    _matrix: *const core::ffi::c_void,
) -> CTFontRef {
    unsafe { CTFontCreateWithName(std::ptr::null(), size, std::ptr::null()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CTFontGetSize(font: CTFontRef) -> CGFloat {
    if font.is_null() { 12.0 } else { unsafe { (*font).size } }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CTFontGetAscent(font: CTFontRef) -> CGFloat {
    let size = unsafe { CTFontGetSize(font) };
    size * 0.8 // approximate ascent
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CTFontGetDescent(font: CTFontRef) -> CGFloat {
    let size = unsafe { CTFontGetSize(font) };
    size * 0.2
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CTFontGetLeading(_font: CTFontRef) -> CGFloat {
    2.0
}

/// Draw a string into a CGContext using the built-in bitmap font.
/// This is our primary text rendering path — called from NSView drawing code.
pub fn draw_text_bitmap(ctx: CGContextRef, text: &str, x: f64, y: f64, color: [f64; 4], scale: f64) {
    if ctx.is_null() || text.is_empty() { return; }
    let c = unsafe { &mut *ctx };
    let rb = (color[0] * 255.0) as u8;
    let gb = (color[1] * 255.0) as u8;
    let bb = (color[2] * 255.0) as u8;
    let ab = (color[3] * 255.0) as u8;

    let char_w = (8.0 * scale) as usize;
    let char_h = (16.0 * scale) as usize;

    for (ci, ch) in text.bytes().enumerate() {
        let bitmap = char_bitmap(ch);
        let cx = x as usize + ci * char_w;

        for row in 0..16 {
            let bits = bitmap[row];
            for col in 0..8 {
                if bits & (0x80 >> col) != 0 {
                    // Scale the pixel
                    let px_x = cx + (col as f64 * scale) as usize;
                    let py_base = y as usize + (row as f64 * scale) as usize;
                    for sy in 0..(scale.ceil() as usize) {
                        for sx in 0..(scale.ceil() as usize) {
                            c.set_pixel(px_x + sx, py_base + sy, rb, gb, bb, ab);
                        }
                    }
                }
            }
        }
    }
}

// ---- CTLine (simplified: just holds a string + font) ----

pub type CTLineRef = *const CTLineInner;

pub struct CTLineInner {
    pub base: CFRuntimeBase,
    pub text: String,
    pub font_size: CGFloat,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CTLineCreateWithAttributedString(
    attr_string: CFTypeRef,
) -> CTLineRef {
    // Extract text from attributed string (simplified: treat as CFString)
    let text = if attr_string.is_null() {
        String::new()
    } else {
        // Try to read as CFString
        let cf = attr_string as *const crate::cf::string::CFStringInner;
        let type_id = unsafe { (*cf).base.type_id() };
        if type_id == CF_STRING_TYPE_ID {
            String::from_utf8_lossy(unsafe { &(*cf).bytes }).into_owned()
        } else {
            String::new()
        }
    };
    Box::into_raw(Box::new(CTLineInner {
        base: CFRuntimeBase::new(41),
        text,
        font_size: 12.0,
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CTLineDraw(line: CTLineRef, ctx: CGContextRef) {
    if line.is_null() || ctx.is_null() { return; }
    let inner = unsafe { &*line };
    // Draw at origin (0,0) — caller should have set CTM
    draw_text_bitmap(ctx, &inner.text, 0.0, 0.0, [0.0, 0.0, 0.0, 1.0], 1.0);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CTLineGetTypographicBounds(
    line: CTLineRef,
    ascent: *mut CGFloat,
    descent: *mut CGFloat,
    leading: *mut CGFloat,
) -> f64 {
    if line.is_null() { return 0.0; }
    let inner = unsafe { &*line };
    if !ascent.is_null() { unsafe { *ascent = inner.font_size * 0.8 }; }
    if !descent.is_null() { unsafe { *descent = inner.font_size * 0.2 }; }
    if !leading.is_null() { unsafe { *leading = 2.0 }; }
    inner.text.len() as f64 * inner.font_size * 0.6 // approximate width
}
