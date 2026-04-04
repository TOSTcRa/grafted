#![allow(non_camel_case_types, non_snake_case)]
//! ObjC ABI type definitions (must match macOS 64-bit layout).

use std::ffi::c_char;

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct objc_selector {
    _private: [u8; 0],
}
pub type SEL = *const objc_selector;

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct objc_object {
    pub isa: Class,
}
pub type id = *mut objc_object;

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct objc_class {
    pub isa: Class,
    pub super_class: Class,
    pub name: *const c_char,
    pub version: libc::c_long,
    pub info: libc::c_long,
    pub instance_size: libc::c_long,
    pub ivars: *mut libc::c_void,       // struct objc_ivar_list *
    pub methodLists: *mut libc::c_void, // struct objc_method_list **
    pub cache: *mut libc::c_void,       // struct objc_cache *
    pub protocols: *mut libc::c_void,   // struct objc_protocol_list *
}
pub type Class = *mut objc_class;

pub type IMP = Option<unsafe extern "C" fn(id, SEL, ...) -> id>;

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct objc_method {
    pub method_name: SEL,
    pub method_types: *const c_char,
    pub method_imp: IMP,
}

/// (simplified for discovery)
#[repr(C)]
pub struct class_t {
    pub isa: *mut class_t,
    pub superclass: *mut class_t,
    pub cache: *mut libc::c_void,
    pub vtable: *mut libc::c_void,
    pub data: *mut class_ro_t,
}

#[repr(C)]
pub struct class_ro_t {
    pub flags: u32,
    pub instance_start: u32,
    pub instance_size: u32,
    pub reserved: u32,
    pub ivar_layout: *const u8,
    pub name: *const c_char,
    pub base_methods: *mut method_list_t,
    pub base_protocols: *mut libc::c_void,
    pub ivars: *mut libc::c_void,
    pub weak_ivar_layout: *const u8,
    pub base_properties: *mut libc::c_void,
}

#[repr(C)]
pub struct method_list_t {
    pub entsize_and_flags: u32,
    pub count: u32,
    // method_t list follows...
}

#[repr(C)]
pub struct method_t {
    pub name: *mut SEL, // In Mach-O this is often a relative offset or pointer to name
    pub types: *const c_char,
    pub imp: IMP,
}
