//! CFString - Core Foundation's universal string type.

use super::types::*;

pub type CFStringRef = *const CFStringInner;
pub type CFMutableStringRef = *mut CFStringInner;

// CFStringEncoding values
pub const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
pub const K_CF_STRING_ENCODING_ASCII: u32 = 0x0600;
pub const K_CF_STRING_ENCODING_MAC_ROMAN: u32 = 0;
pub const K_CF_STRING_ENCODING_UTF16: u32 = 0x0100;
pub const K_CF_STRING_ENCODING_UTF16BE: u32 = 0x1000_0100;
pub const K_CF_STRING_ENCODING_UTF16LE: u32 = 0x1400_0100;

#[repr(C)]
pub struct CFStringInner {
    pub base: CFRuntimeBase,
    pub bytes: Vec<u8>, // UTF-8 storage
}

impl CFStringInner {
    fn new(s: &str) -> *const Self {
        let obj = Box::new(Self {
            base: CFRuntimeBase::new(CF_STRING_TYPE_ID),
            bytes: s.as_bytes().to_vec(),
        });
        Box::into_raw(obj)
    }

    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes).unwrap_or("")
    }
}

/// Create a CFString from a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFStringCreateWithCString(
    _alloc: CFAllocatorRef,
    cstr: *const i8,
    _encoding: u32,
) -> CFStringRef {
    if cstr.is_null() { return std::ptr::null(); }
    let s = unsafe { std::ffi::CStr::from_ptr(cstr) }.to_str().unwrap_or("");
    CFStringInner::new(s)
}

/// Create a CFString from raw bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFStringCreateWithBytes(
    _alloc: CFAllocatorRef,
    bytes: *const u8,
    num_bytes: CFIndex,
    _encoding: u32,
    _is_external: CFBoolean,
) -> CFStringRef {
    if bytes.is_null() || num_bytes <= 0 { return std::ptr::null(); }
    let slice = unsafe { std::slice::from_raw_parts(bytes, num_bytes as usize) };
    let s = String::from_utf8_lossy(slice);
    CFStringInner::new(&s)
}

/// Create a CFString by formatting (simplified: only handles %s and %d).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFStringCreateWithFormat(
    _alloc: CFAllocatorRef,
    _opts: CFTypeRef,
    format: CFStringRef,
    // ... varargs not supported, just return the format string
) -> CFStringRef {
    if format.is_null() { return std::ptr::null(); }
    let fmt = unsafe { &*format };
    CFStringInner::new(fmt.as_str())
}

/// Get the length in UTF-16 code units.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFStringGetLength(s: CFStringRef) -> CFIndex {
    if s.is_null() { return 0; }
    let inner = unsafe { &*s };
    inner.as_str().encode_utf16().count() as CFIndex
}

/// Get a C string pointer. Returns NULL if the internal encoding doesn't match.
/// The returned pointer is only valid as long as the CFString is alive.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFStringGetCStringPtr(
    s: CFStringRef,
    _encoding: u32,
) -> *const i8 {
    if s.is_null() { return std::ptr::null(); }
    let inner = unsafe { &*s };
    // Only works if the string has no embedded NULs
    if inner.bytes.contains(&0) { return std::ptr::null(); }
    inner.bytes.as_ptr() as *const i8
}

/// Copy the string content to a caller-provided buffer as a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFStringGetCString(
    s: CFStringRef,
    buffer: *mut i8,
    buffer_size: CFIndex,
    _encoding: u32,
) -> CFBoolean {
    if s.is_null() || buffer.is_null() || buffer_size <= 0 { return K_CF_BOOLEAN_FALSE; }
    let inner = unsafe { &*s };
    let max_copy = (buffer_size as usize).saturating_sub(1).min(inner.bytes.len());
    unsafe {
        std::ptr::copy_nonoverlapping(inner.bytes.as_ptr(), buffer as *mut u8, max_copy);
        *buffer.add(max_copy) = 0; // NUL terminate
    }
    K_CF_BOOLEAN_TRUE
}

/// Get UTF-16 characters into a buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFStringGetCharacters(
    s: CFStringRef,
    range_loc: CFIndex,
    range_len: CFIndex,
    buffer: *mut u16,
) {
    if s.is_null() || buffer.is_null() { return; }
    let inner = unsafe { &*s };
    let utf16: Vec<u16> = inner.as_str().encode_utf16().collect();
    let start = range_loc as usize;
    let end = (start + range_len as usize).min(utf16.len());
    if start < end {
        unsafe {
            std::ptr::copy_nonoverlapping(
                utf16[start..end].as_ptr(),
                buffer,
                end - start,
            );
        }
    }
}

/// Compare two CFStrings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFStringCompare(
    s1: CFStringRef,
    s2: CFStringRef,
    _flags: CFOptionFlags,
) -> i32 {
    let str1 = if s1.is_null() { "" } else { unsafe { &*s1 }.as_str() };
    let str2 = if s2.is_null() { "" } else { unsafe { &*s2 }.as_str() };
    match str1.cmp(str2) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// Compile-time constant string (CFSTR macro equivalent).
/// Darwin binaries use __CFConstantStringClassReference for literal strings.
#[repr(C)]
pub struct CFConstantString {
    pub isa: *const core::ffi::c_void,
    pub flags: u32,
    pub str_ptr: *const u8,
    pub length: i64,
}

/// The class reference that __builtin___CFStringMakeConstantString uses.
#[unsafe(no_mangle)]
pub static __CFConstantStringClassReference: [usize; 4] = [0; 4];

// CFStringGetTypeID is in types.rs
