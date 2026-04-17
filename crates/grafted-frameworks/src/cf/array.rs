//! CFArray - ordered collection of CFTypeRef values.

use super::types::*;

pub type CFArrayRef = *const CFArrayInner;

pub type CFArrayCallBacks = *const core::ffi::c_void;

#[repr(C)]
pub struct CFArrayInner {
    pub base: CFRuntimeBase,
    pub values: Vec<CFTypeRef>,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFArrayCreate(
    _alloc: CFAllocatorRef,
    values: *const CFTypeRef,
    num: CFIndex,
    _cbs: CFArrayCallBacks,
) -> CFArrayRef {
    let mut vec = Vec::with_capacity(num.max(0) as usize);
    if !values.is_null() && num > 0 {
        for i in 0..num as usize {
            let v = unsafe { *values.add(i) };
            if !v.is_null() { unsafe { CFRetain(v) }; }
            vec.push(v);
        }
    }
    Box::into_raw(Box::new(CFArrayInner {
        base: CFRuntimeBase::new(CF_ARRAY_TYPE_ID),
        values: vec,
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFArrayCreateMutable(
    _alloc: CFAllocatorRef,
    capacity: CFIndex,
    _cbs: CFArrayCallBacks,
) -> *mut CFArrayInner {
    Box::into_raw(Box::new(CFArrayInner {
        base: CFRuntimeBase::new(CF_ARRAY_TYPE_ID),
        values: Vec::with_capacity(capacity.max(0) as usize),
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFArrayGetCount(arr: CFArrayRef) -> CFIndex {
    if arr.is_null() { return 0; }
    unsafe { (*arr).values.len() as CFIndex }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFArrayGetValueAtIndex(arr: CFArrayRef, idx: CFIndex) -> CFTypeRef {
    if arr.is_null() { return K_CF_NULL; }
    let inner = unsafe { &*arr };
    inner.values.get(idx as usize).copied().unwrap_or(K_CF_NULL)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFArrayAppendValue(arr: *mut CFArrayInner, value: CFTypeRef) {
    if arr.is_null() { return; }
    if !value.is_null() { unsafe { CFRetain(value) }; }
    unsafe { (*arr).values.push(value) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFArrayGetFirstIndexOfValue(
    arr: CFArrayRef,
    _range_loc: CFIndex,
    _range_len: CFIndex,
    value: CFTypeRef,
) -> CFIndex {
    if arr.is_null() { return K_CF_NOT_FOUND; }
    let inner = unsafe { &*arr };
    inner.values.iter().position(|v| *v == value)
        .map(|i| i as CFIndex)
        .unwrap_or(K_CF_NOT_FOUND)
}

#[unsafe(no_mangle)]
pub static kCFTypeArrayCallBacks: [usize; 5] = [0; 5];
