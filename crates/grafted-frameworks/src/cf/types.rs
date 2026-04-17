//! CF type system - reference counting, type IDs, CFTypeRef.

use std::sync::atomic::{AtomicU32, Ordering};

/// Opaque pointer to any CF object.
pub type CFTypeRef = *const core::ffi::c_void;
pub type CFMutableTypeRef = *mut core::ffi::c_void;

pub type CFTypeID = u64;
pub type CFIndex = i64;
pub type CFHashCode = u64;
pub type CFOptionFlags = u64;
pub type CFBoolean = u8;

pub const K_CF_NULL: CFTypeRef = core::ptr::null();
pub const K_CF_BOOLEAN_TRUE: CFBoolean = 1;
pub const K_CF_BOOLEAN_FALSE: CFBoolean = 0;
pub const K_CF_NOT_FOUND: CFIndex = -1;

// Type IDs (stable, matching Darwin layout for toll-free bridging)
pub const CF_STRING_TYPE_ID: CFTypeID = 7;
pub const CF_DICTIONARY_TYPE_ID: CFTypeID = 18;
pub const CF_ARRAY_TYPE_ID: CFTypeID = 19;
pub const CF_DATA_TYPE_ID: CFTypeID = 20;
pub const CF_BOOLEAN_TYPE_ID: CFTypeID = 21;
pub const CF_NUMBER_TYPE_ID: CFTypeID = 22;
pub const CF_RUNLOOP_TYPE_ID: CFTypeID = 23;
pub const CF_RUNLOOP_SOURCE_TYPE_ID: CFTypeID = 24;
pub const CF_RUNLOOP_TIMER_TYPE_ID: CFTypeID = 25;

/// Header at the start of every CF object.
#[repr(C)]
pub struct CFRuntimeBase {
    /// ObjC ISA pointer for toll-free bridging (NULL if not bridged)
    pub isa: *const core::ffi::c_void,
    /// Type ID in lower 16 bits, flags in upper bits
    pub info: u64,
    /// Reference count (starts at 1)
    pub refcount: AtomicU32,
    _pad: u32,
}

impl CFRuntimeBase {
    pub fn new(type_id: CFTypeID) -> Self {
        Self {
            isa: core::ptr::null(),
            info: type_id & 0xFFFF,
            refcount: AtomicU32::new(1),
            _pad: 0,
        }
    }

    pub fn type_id(&self) -> CFTypeID {
        self.info & 0xFFFF
    }

    pub fn retain(&self) {
        self.refcount.fetch_add(1, Ordering::Relaxed);
    }

    /// Returns true if the object should be deallocated.
    pub fn release(&self) -> bool {
        self.refcount.fetch_sub(1, Ordering::Release) == 1
    }
}

/// Get the type ID from any CFTypeRef.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFGetTypeID(cf: CFTypeRef) -> CFTypeID {
    if cf.is_null() { return 0; }
    let base = cf as *const CFRuntimeBase;
    unsafe { (*base).type_id() }
}

/// Increment the reference count of a CF object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFRetain(cf: CFTypeRef) -> CFTypeRef {
    if !cf.is_null() {
        let base = cf as *const CFRuntimeBase;
        unsafe { (*base).retain() };
    }
    cf
}

/// Decrement the reference count. Frees the object when it reaches 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFRelease(cf: CFTypeRef) {
    if cf.is_null() { return; }
    let base = cf as *const CFRuntimeBase;
    if unsafe { (*base).release() } {
        // Synchronize with the release
        std::sync::atomic::fence(Ordering::Acquire);
        // Dispatch to type-specific destructor
        let type_id = unsafe { (*base).type_id() };
        match type_id {
            CF_STRING_TYPE_ID => {
                let _ = unsafe { Box::from_raw(cf as *mut super::string::CFStringInner) };
            }
            CF_DICTIONARY_TYPE_ID => {
                let _ = unsafe { Box::from_raw(cf as *mut super::dictionary::CFDictionaryInner) };
            }
            CF_ARRAY_TYPE_ID => {
                let _ = unsafe { Box::from_raw(cf as *mut super::array::CFArrayInner) };
            }
            CF_DATA_TYPE_ID => {
                let _ = unsafe { Box::from_raw(cf as *mut super::data::CFDataInner) };
            }
            _ => {
                // Unknown type - just drop the base allocation
                let _ = unsafe { Box::from_raw(cf as *mut CFRuntimeBase) };
            }
        }
    }
}

/// Returns true if two CF objects are equal.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFEqual(cf1: CFTypeRef, cf2: CFTypeRef) -> CFBoolean {
    if cf1 == cf2 { return K_CF_BOOLEAN_TRUE; }
    if cf1.is_null() || cf2.is_null() { return K_CF_BOOLEAN_FALSE; }
    let t1 = unsafe { CFGetTypeID(cf1) };
    let t2 = unsafe { CFGetTypeID(cf2) };
    if t1 != t2 { return K_CF_BOOLEAN_FALSE; }
    // Type-specific equality
    match t1 {
        CF_STRING_TYPE_ID => {
            let s1 = unsafe { &*(cf1 as *const super::string::CFStringInner) };
            let s2 = unsafe { &*(cf2 as *const super::string::CFStringInner) };
            if s1.bytes == s2.bytes { K_CF_BOOLEAN_TRUE } else { K_CF_BOOLEAN_FALSE }
        }
        _ => K_CF_BOOLEAN_FALSE,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFHash(cf: CFTypeRef) -> CFHashCode {
    if cf.is_null() { return 0; }
    // Simple hash: use pointer address
    cf as CFHashCode
}

// Type ID getters
#[unsafe(no_mangle)] pub extern "C" fn CFStringGetTypeID() -> CFTypeID { CF_STRING_TYPE_ID }
#[unsafe(no_mangle)] pub extern "C" fn CFDictionaryGetTypeID() -> CFTypeID { CF_DICTIONARY_TYPE_ID }
#[unsafe(no_mangle)] pub extern "C" fn CFArrayGetTypeID() -> CFTypeID { CF_ARRAY_TYPE_ID }
#[unsafe(no_mangle)] pub extern "C" fn CFDataGetTypeID() -> CFTypeID { CF_DATA_TYPE_ID }
#[unsafe(no_mangle)] pub extern "C" fn CFBooleanGetTypeID() -> CFTypeID { CF_BOOLEAN_TYPE_ID }
#[unsafe(no_mangle)] pub extern "C" fn CFNumberGetTypeID() -> CFTypeID { CF_NUMBER_TYPE_ID }

// Allocator stubs - always use the default allocator
pub type CFAllocatorRef = *const core::ffi::c_void;
pub const K_CF_ALLOCATOR_DEFAULT: CFAllocatorRef = core::ptr::null();

#[unsafe(no_mangle)]
pub extern "C" fn CFAllocatorGetDefault() -> CFAllocatorRef { K_CF_ALLOCATOR_DEFAULT }
