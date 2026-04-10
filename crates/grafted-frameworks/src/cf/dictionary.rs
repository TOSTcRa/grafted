//! CFDictionary — key-value store used throughout Darwin for configuration,
//! property lists, user defaults, and framework communication.

use std::collections::HashMap;
use super::types::*;

pub type CFDictionaryRef = *const CFDictionaryInner;

// Callback types (simplified — we use pointer equality for keys)
pub type CFDictionaryKeyCallBacks = *const core::ffi::c_void;
pub type CFDictionaryValueCallBacks = *const core::ffi::c_void;

#[repr(C)]
pub struct CFDictionaryInner {
    pub base: CFRuntimeBase,
    pub entries: HashMap<usize, CFTypeRef>, // key pointer → value pointer
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFDictionaryCreate(
    _alloc: CFAllocatorRef,
    keys: *const CFTypeRef,
    values: *const CFTypeRef,
    num: CFIndex,
    _key_cbs: CFDictionaryKeyCallBacks,
    _val_cbs: CFDictionaryValueCallBacks,
) -> CFDictionaryRef {
    let mut entries = HashMap::new();
    if !keys.is_null() && !values.is_null() && num > 0 {
        for i in 0..num as usize {
            let k = unsafe { *keys.add(i) } as usize;
            let v = unsafe { *values.add(i) };
            if !v.is_null() {
                unsafe { CFRetain(v) };
            }
            entries.insert(k, v);
        }
    }
    let obj = Box::new(CFDictionaryInner {
        base: CFRuntimeBase::new(CF_DICTIONARY_TYPE_ID),
        entries,
    });
    Box::into_raw(obj)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFDictionaryCreateMutable(
    _alloc: CFAllocatorRef,
    capacity: CFIndex,
    _key_cbs: CFDictionaryKeyCallBacks,
    _val_cbs: CFDictionaryValueCallBacks,
) -> *mut CFDictionaryInner {
    let obj = Box::new(CFDictionaryInner {
        base: CFRuntimeBase::new(CF_DICTIONARY_TYPE_ID),
        entries: HashMap::with_capacity(capacity.max(0) as usize),
    });
    Box::into_raw(obj)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFDictionaryGetCount(dict: CFDictionaryRef) -> CFIndex {
    if dict.is_null() { return 0; }
    unsafe { (*dict).entries.len() as CFIndex }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFDictionaryGetValue(
    dict: CFDictionaryRef,
    key: CFTypeRef,
) -> CFTypeRef {
    if dict.is_null() { return K_CF_NULL; }
    let inner = unsafe { &*dict };
    inner.entries.get(&(key as usize)).copied().unwrap_or(K_CF_NULL)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFDictionaryContainsKey(
    dict: CFDictionaryRef,
    key: CFTypeRef,
) -> CFBoolean {
    if dict.is_null() { return K_CF_BOOLEAN_FALSE; }
    let inner = unsafe { &*dict };
    if inner.entries.contains_key(&(key as usize)) { K_CF_BOOLEAN_TRUE } else { K_CF_BOOLEAN_FALSE }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFDictionarySetValue(
    dict: *mut CFDictionaryInner,
    key: CFTypeRef,
    value: CFTypeRef,
) {
    if dict.is_null() { return; }
    if !value.is_null() {
        unsafe { CFRetain(value) };
    }
    let inner = unsafe { &mut *dict };
    // Release old value if present
    if let Some(old) = inner.entries.insert(key as usize, value) {
        if !old.is_null() {
            unsafe { CFRelease(old) };
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFDictionaryRemoveValue(
    dict: *mut CFDictionaryInner,
    key: CFTypeRef,
) {
    if dict.is_null() { return; }
    let inner = unsafe { &mut *dict };
    if let Some(old) = inner.entries.remove(&(key as usize)) {
        if !old.is_null() {
            unsafe { CFRelease(old) };
        }
    }
}

// Standard callback constants (stubs — we use pointer identity)
#[unsafe(no_mangle)]
pub static kCFTypeDictionaryKeyCallBacks: [usize; 6] = [0; 6];
#[unsafe(no_mangle)]
pub static kCFTypeDictionaryValueCallBacks: [usize; 5] = [0; 5];
#[unsafe(no_mangle)]
pub static kCFCopyStringDictionaryKeyCallBacks: [usize; 6] = [0; 6];
