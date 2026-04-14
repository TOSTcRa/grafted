#![allow(non_snake_case)]

use std::arch::global_asm;
use std::collections::HashMap;
use std::sync::RwLock;

use crate::types::{id, SEL, IMP, Class, class_t};

lazy_static::lazy_static! {
    static ref METHOD_REGISTRY: RwLock<HashMap<(usize, usize), IMP>> = RwLock::new(HashMap::new());
    static ref CLASS_REGISTRY: RwLock<HashMap<String, usize>> = RwLock::new(HashMap::new());
    static ref SELECTOR_REGISTRY: RwLock<HashMap<String, usize>> = RwLock::new(HashMap::new());
}

pub fn class_addMethod(cls: Class, name: SEL, imp: IMP, _types: *const std::ffi::c_char) -> bool {
    let mut reg = METHOD_REGISTRY.write().unwrap();
    let key = (cls as usize, name as usize);
    if reg.contains_key(&key) {
        return false;
    }
    reg.insert(key, imp);
    true
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_registerClassPair(cls: Class) {
    if cls.is_null() { return; }
    unsafe {
        let class_t_ptr = cls as *mut class_t;
        // Strip ObjC tag bits (low 3 bits) from class_data_bits pointer.
        // Our malloc'd classes have low bits = 0, so this is safe for all cases.
        let raw_data = (*class_t_ptr).data as usize;
        let data_ptr = (raw_data & !7) as *mut crate::types::class_ro_t;
        if data_ptr.is_null() { return; }

        let name_ptr = (*data_ptr).name;
        if !name_ptr.is_null() {
            let name = std::ffi::CStr::from_ptr(name_ptr).to_string_lossy().into_owned();
            let mut reg = CLASS_REGISTRY.write().unwrap();
            reg.insert(name, cls as usize);
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn objc_getClass(name: *const std::ffi::c_char) -> Class {
    if name.is_null() { return std::ptr::null_mut(); }
    let name_str = unsafe { std::ffi::CStr::from_ptr(name) }.to_string_lossy().into_owned();
    let reg = CLASS_REGISTRY.read().unwrap();
    let ptr = reg.get(&name_str).copied().unwrap_or(0);
    ptr as Class
}

#[unsafe(no_mangle)]
pub extern "C" fn sel_registerName(name: *const std::ffi::c_char) -> SEL {
    if name.is_null() { return std::ptr::null(); }
    let name_str = unsafe { std::ffi::CStr::from_ptr(name) }.to_string_lossy().into_owned();
    
    let mut reg = SELECTOR_REGISTRY.write().unwrap();
    if let Some(&sel) = reg.get(&name_str) {
        return sel as SEL;
    }
    
    // Store with null terminator so CStr::from_ptr works in grafted_lookup_method
    let mut bytes = name_str.into_bytes();
    bytes.push(0);
    let b = bytes.into_boxed_slice();
    let ptr = Box::into_raw(b) as *const u8 as SEL;
    reg.insert(unsafe { std::ffi::CStr::from_ptr(name) }.to_string_lossy().into_owned(), ptr as usize);
    ptr
}

// Built-in methods for root objects
#[unsafe(no_mangle)]
extern "C" fn grafted_alloc(cls: id, _cmd: SEL) -> id {
    if cls.is_null() { return std::ptr::null_mut(); }
    unsafe {
        // Use a generous fixed allocation. Reading instance_size from class metadata
        // is unreliable because translate_swift_metadata shifts the Darwin layout.
        let ptr = libc::calloc(1, 512) as id;
        if !ptr.is_null() {
            (*ptr).isa = cls as Class;
            // Set Swift-compatible refcount at +8 (immortal so retain/release don't crash)
            *((ptr as *mut u64).add(1)) = 0xFFFFFFFFFFFFFFFF; // immortal refcount
        }
        ptr
    }
}

#[unsafe(no_mangle)]
extern "C" fn grafted_init(obj: id, _cmd: SEL) -> id {
    obj
}

#[unsafe(no_mangle)]
pub extern "C" fn grafted_lookup_method(receiver: id, selector: SEL) -> IMP {
    if receiver.is_null() {
        return None;
    }

    // Try exact pointer match first (fast path), then string-based fallback.
    // Binary selectors come from __objc_selrefs (binary data) while our
    // registered selectors come from sel_registerName (heap) — different pointers
    // for the same string. We need string comparison as fallback.
    // Check receiver itself (class method dispatch) and receiver.isa (instance dispatch).
    // Guard against bad isa pointers from binary objects with translated metadata.
    let isa = unsafe { (*receiver).isa };
    let isa_addr = isa as usize;
    let safe_isa = if isa_addr > 0x1000 && isa_addr < 0x800000000000 { isa } else { std::ptr::null_mut() };
    let classes_to_check: [Class; 2] = [receiver as Class, safe_isa];

    // Fast path: exact (cls, sel) pointer match
    {
        let reg = METHOD_REGISTRY.read().unwrap();
        for &check_cls in &classes_to_check {
            if check_cls.is_null() { continue; }
            let key = (check_cls as usize, selector as usize);
            if let Some(&imp) = reg.get(&key) {
                return imp;
            }
        }
    }

    // The binary may use its own selector pointer (from __objc_selrefs) which differs
    // from our sel_registerName pointer. Look up the selector string in our registry.
    // Only do this if the selector pointer looks like a valid string address.
    let sel_addr = selector as usize;
    if sel_addr > 0x1000 {
        // Check if this selector string already exists in SELECTOR_REGISTRY
        let sel_str = unsafe { std::ffi::CStr::from_ptr(selector as *const i8) };
        if let Ok(name) = sel_str.to_str() {
            if name.len() < 256 { // sanity check
                let sreg = SELECTOR_REGISTRY.read().unwrap();
                if let Some(&canonical_addr) = sreg.get(name) {
                    let canonical_sel = canonical_addr as SEL;
                    if canonical_sel != selector {
                        let reg = METHOD_REGISTRY.read().unwrap();
                        for &check_cls in &classes_to_check {
                            if check_cls.is_null() { continue; }
                            let key = (check_cls as usize, canonical_sel as usize);
                            if let Some(&imp) = reg.get(&key) {
                                return imp;
                            }
                        }
                    }
                }
            }
        }
    }

    let sel_name = unsafe { std::ffi::CStr::from_ptr(selector as *const i8).to_string_lossy() };
    if sel_name == "alloc" || sel_name == "allocWithZone:" {
        return Some(unsafe { std::mem::transmute(grafted_alloc as *const ()) });
    }
    if sel_name == "init" || sel_name == "new" {
        return Some(unsafe { std::mem::transmute(grafted_init as *const ()) });
    }
    if sel_name == "retain" || sel_name == "autorelease" || sel_name == "self" {
        return Some(unsafe { std::mem::transmute(grafted_init as *const ()) }); // returns self
    }
    if sel_name == "release" || sel_name == "dealloc" {
        return Some(unsafe { std::mem::transmute(grafted_noop as *const ()) });
    }
    if sel_name == "respondsToSelector:" || sel_name == "conformsToProtocol:"
        || sel_name == "isKindOfClass:" || sel_name == "isMemberOfClass:" {
        return Some(unsafe { std::mem::transmute(grafted_returns_false as *const ()) });
    }
    if sel_name == "class" || sel_name == "superclass" || sel_name == "description"
        || sel_name == "debugDescription" {
        return Some(unsafe { std::mem::transmute(grafted_returns_null as *const ()) });
    }
    if sel_name == "hash" || sel_name == "count" || sel_name == "length" {
        return Some(unsafe { std::mem::transmute(grafted_returns_zero as *const ()) });
    }

    // Log unknown selectors for debugging (first N)
    static UNHANDLED_COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = UNHANDLED_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if n < 20 {
        // Get class name if possible
        let cls_name = get_class_name(classes_to_check[1]); // isa
        let cls_name = if cls_name.starts_with("0x") { get_class_name(classes_to_check[0]) } else { cls_name };
        log::warn!("objc: unhandled [{cls_name} {sel_name}]");
    }

    // Return a soft no-op stub instead of crashing
    Some(unsafe { std::mem::transmute(grafted_returns_null as *const ()) })
}

fn get_class_name(cls: Class) -> String {
    if cls.is_null() { return "?".into(); }
    let reg = CLASS_REGISTRY.read().unwrap();
    for (name, &addr) in reg.iter() {
        if addr == cls as usize {
            return name.clone();
        }
    }
    format!("{:p}", cls)
}

unsafe extern "C" fn grafted_noop(_self: id, _sel: SEL) {}
unsafe extern "C" fn grafted_returns_false(_self: id, _sel: SEL) -> i32 { 0 }
unsafe extern "C" fn grafted_returns_null(_self: id, _sel: SEL) -> *mut u8 { std::ptr::null_mut() }
unsafe extern "C" fn grafted_returns_zero(_self: id, _sel: SEL) -> u64 { 0 }

// objc_msgSend implementation in naked assembly.
// Must save argument registers, call grafted_lookup_method, restore registers,
// and tail-call (jmp) to the returned IMP (in rax).
global_asm!(r#"
    .global objc_msgSend
    .type objc_msgSend, @function
objc_msgSend:
    // If receiver (rdi) is nil, return 0 (rax)
    test rdi, rdi
    jz .Lnil_receiver

    // Save integer arguments
    push rbp
    mov rbp, rsp
    push rdi
    push rsi
    push rdx
    push rcx
    push r8
    push r9

    // Save floating-point arguments (xmm0-xmm7)
    sub rsp, 128
    movups [rsp + 0x00], xmm0
    movups [rsp + 0x10], xmm1
    movups [rsp + 0x20], xmm2
    movups [rsp + 0x30], xmm3
    movups [rsp + 0x40], xmm4
    movups [rsp + 0x50], xmm5
    movups [rsp + 0x60], xmm6
    movups [rsp + 0x70], xmm7

    // Call grafted_lookup_method(rdi: id, rsi: SEL) -> IMP
    call grafted_lookup_method

    // The IMP is now in rax. If it's null (rare, handled inside lookup), trap.
    test rax, rax
    jz .Lnil_receiver

    // Restore xmm registers
    movups xmm0, [rsp + 0x00]
    movups xmm1, [rsp + 0x10]
    movups xmm2, [rsp + 0x20]
    movups xmm3, [rsp + 0x30]
    movups xmm4, [rsp + 0x40]
    movups xmm5, [rsp + 0x50]
    movups xmm6, [rsp + 0x60]
    movups xmm7, [rsp + 0x70]
    add rsp, 128

    // Restore integer registers
    pop r9
    pop r8
    pop rcx
    pop rdx
    pop rsi
    pop rdi

    pop rbp

    // Tail call to the method implementation
    jmp rax

.Lnil_receiver:
    xor rax, rax
    // Zero out other potential return registers
    xor rdx, rdx
    ret
"#);

unsafe extern "C" {
    pub fn objc_msgSend(receiver: id, selector: SEL, ...) -> id;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::objc_class;

    unsafe extern "C" fn mock_method(_self_ptr: id, _cmd: SEL, arg1: usize) -> id {
        // Return arg1 + 42 as a pointer to verify arguments are passed correctly.
        (arg1 + 42) as id
    }

    #[test]
    fn test_objc_msgsend() {
        let mut cls = objc_class {
            isa: std::ptr::null_mut(),
            super_class: std::ptr::null_mut(),
            name: std::ptr::null(),
            version: 0,
            info: 0,
            instance_size: 0,
            ivars: std::ptr::null_mut(),
            methodLists: std::ptr::null_mut(),
            cache: std::ptr::null_mut(),
            protocols: std::ptr::null_mut(),
        };
        let cls_ptr = &mut cls as *mut _ as Class;

        let mut obj = crate::types::objc_object { isa: cls_ptr };
        let obj_ptr = &mut obj as *mut _ as id;

        let sel = 0xdeadbeefusize as SEL;

        class_addMethod(cls_ptr, sel, Some(unsafe { std::mem::transmute(mock_method as *const ()) }), std::ptr::null());

        let result = unsafe { objc_msgSend(obj_ptr, sel, 100usize) };
        assert_eq!(result as usize, 142);
    }
}
