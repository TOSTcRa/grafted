//! CFData - immutable/mutable byte buffer.

use super::types::*;

pub type CFDataRef = *const CFDataInner;

#[repr(C)]
pub struct CFDataInner {
    pub base: CFRuntimeBase,
    pub bytes: Vec<u8>,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFDataCreate(
    _alloc: CFAllocatorRef,
    bytes: *const u8,
    length: CFIndex,
) -> CFDataRef {
    let data = if bytes.is_null() || length <= 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(bytes, length as usize) }.to_vec()
    };
    Box::into_raw(Box::new(CFDataInner {
        base: CFRuntimeBase::new(CF_DATA_TYPE_ID),
        bytes: data,
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFDataGetLength(data: CFDataRef) -> CFIndex {
    if data.is_null() { return 0; }
    unsafe { (*data).bytes.len() as CFIndex }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFDataGetBytePtr(data: CFDataRef) -> *const u8 {
    if data.is_null() { return std::ptr::null(); }
    unsafe { (*data).bytes.as_ptr() }
}
