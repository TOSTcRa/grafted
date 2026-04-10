//! CGColorSpace and CGColor.

use super::geometry::CGFloat;
use crate::cf::types::*;

pub type CGColorSpaceRef = *const CGColorSpaceInner;
pub type CGColorRef = *const CGColorInner;

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CGColorSpaceModel {
    Unknown = -1_i32 as u32,
    Monochrome = 0,
    RGB = 1,
    CMYK = 2,
    Lab = 3,
    DeviceN = 4,
    Indexed = 5,
    Pattern = 6,
}

pub struct CGColorSpaceInner {
    pub base: CFRuntimeBase,
    pub model: CGColorSpaceModel,
    pub components: usize,
}

pub struct CGColorInner {
    pub base: CFRuntimeBase,
    pub components: [CGFloat; 4], // RGBA
    pub color_space: CGColorSpaceRef,
}

#[unsafe(no_mangle)]
pub extern "C" fn CGColorSpaceCreateDeviceRGB() -> CGColorSpaceRef {
    Box::into_raw(Box::new(CGColorSpaceInner {
        base: CFRuntimeBase::new(30), // CG type IDs start at 30
        model: CGColorSpaceModel::RGB,
        components: 3,
    }))
}

#[unsafe(no_mangle)]
pub extern "C" fn CGColorSpaceCreateDeviceGray() -> CGColorSpaceRef {
    Box::into_raw(Box::new(CGColorSpaceInner {
        base: CFRuntimeBase::new(30),
        model: CGColorSpaceModel::Monochrome,
        components: 1,
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CGColorSpaceGetNumberOfComponents(cs: CGColorSpaceRef) -> usize {
    if cs.is_null() { return 0; }
    unsafe { (*cs).components }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CGColorSpaceGetModel(cs: CGColorSpaceRef) -> i32 {
    if cs.is_null() { return -1; }
    unsafe { (*cs).model as i32 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CGColorSpaceRelease(cs: CGColorSpaceRef) {
    if !cs.is_null() { let _ = unsafe { Box::from_raw(cs as *mut CGColorSpaceInner) }; }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CGColorCreate(
    color_space: CGColorSpaceRef,
    components: *const CGFloat,
) -> CGColorRef {
    let mut rgba = [0.0f64; 4];
    if !components.is_null() {
        let n = if color_space.is_null() { 4 } else { unsafe { (*color_space).components + 1 } };
        for i in 0..n.min(4) {
            rgba[i] = unsafe { *components.add(i) };
        }
    }
    Box::into_raw(Box::new(CGColorInner {
        base: CFRuntimeBase::new(31),
        components: rgba,
        color_space,
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CGColorGetComponents(color: CGColorRef) -> *const CGFloat {
    if color.is_null() { return std::ptr::null(); }
    unsafe { (*color).components.as_ptr() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CGColorRelease(color: CGColorRef) {
    if !color.is_null() { let _ = unsafe { Box::from_raw(color as *mut CGColorInner) }; }
}
