//! Framework symbol registry — maps Darwin framework paths to our implementations.
//!
//! When a Darwin binary loads `/System/Library/Frameworks/AppKit.framework/...`,
//! the linker calls `framework_symbols()` to get our implementation addresses.

use std::collections::HashMap;

/// Returns a map: framework_path → { symbol_name → address }.
/// The linker merges these into its symbol registry.
pub fn framework_registry() -> HashMap<String, HashMap<String, u64>> {
    let mut reg = HashMap::new();

    reg.insert(
        "/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation".into(),
        core_foundation_symbols(),
    );
    reg.insert(
        "/System/Library/Frameworks/Foundation.framework/Versions/C/Foundation".into(),
        foundation_symbols(),
    );
    reg.insert(
        "/System/Library/Frameworks/CoreGraphics.framework/Versions/A/CoreGraphics".into(),
        core_graphics_symbols(),
    );
    reg.insert(
        "/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit".into(),
        appkit_symbols(),
    );
    reg.insert(
        "/System/Library/Frameworks/ApplicationServices.framework/Versions/A/ApplicationServices".into(),
        core_graphics_symbols(), // ApplicationServices includes CG
    );
    reg.insert(
        "/System/Library/Frameworks/Carbon.framework/Versions/A/Carbon".into(),
        carbon_symbols(),
    );
    reg.insert(
        "/System/Library/Frameworks/QuartzCore.framework/Versions/A/QuartzCore".into(),
        quartz_core_symbols(),
    );
    reg.insert(
        "/System/Library/Frameworks/CoreServices.framework/Versions/A/CoreServices".into(),
        core_services_symbols(),
    );

    // Swift runtime
    for lib in &[
        "/usr/lib/swift/libswiftCore.dylib",
        "/usr/lib/swift/libswiftFoundation.dylib",
        "/usr/lib/swift/libswiftCoreFoundation.dylib",
        "/usr/lib/swift/libswiftCoreGraphics.dylib",
        "/usr/lib/swift/libswiftDarwin.dylib",
        "/usr/lib/swift/libswiftDispatch.dylib",
        "/usr/lib/swift/libswiftObjectiveC.dylib",
        "/usr/lib/swift/libswiftOSLog.dylib",
        "/usr/lib/swift/libswiftos.dylib",
        "/usr/lib/swift/libswiftMetal.dylib",
        "/usr/lib/swift/libswiftIOKit.dylib",
        "/usr/lib/swift/libswiftsimd.dylib",
        "/usr/lib/swift/libswiftSpatial.dylib",
        "/usr/lib/swift/libswiftQuartzCore.dylib",
        "/usr/lib/swift/libswiftObservation.dylib",
        "/usr/lib/swift/libswiftAccelerate.dylib",
        "/usr/lib/swift/libswiftAVFoundation.dylib",
        "/usr/lib/swift/libswiftCoreAudio.dylib",
        "/usr/lib/swift/libswiftCoreImage.dylib",
        "/usr/lib/swift/libswiftCoreLocation.dylib",
        "/usr/lib/swift/libswiftCoreMedia.dylib",
        "/usr/lib/swift/libswiftCoreMIDI.dylib",
        "/usr/lib/swift/libswiftXPC.dylib",
        "/usr/lib/swift/libswiftUniformTypeIdentifiers.dylib",
        "/usr/lib/swift/libswift_Concurrency.dylib",
    ] {
        reg.insert((*lib).into(), swift_runtime_symbols());
    }

    // Other system libraries
    reg.insert("/usr/lib/libc++.1.dylib".into(), libcxx_symbols());
    reg.insert("/usr/lib/libc++abi.dylib".into(), libcxx_symbols());

    // Stub frameworks with empty symbol tables (prevents load errors)
    for path in &[
        "/System/Library/Frameworks/SwiftUI.framework/Versions/A/SwiftUI",
        "/System/Library/Frameworks/SwiftData.framework/Versions/A/SwiftData",
        "/System/Library/Frameworks/_SwiftData_SwiftUI.framework/Versions/A/_SwiftData_SwiftUI",
        "/System/Library/Frameworks/Combine.framework/Versions/A/Combine",
        "/System/Library/Frameworks/Vision.framework/Versions/A/Vision",
        "/System/Library/Frameworks/UserNotifications.framework/Versions/A/UserNotifications",
        "/System/Library/Frameworks/ServiceManagement.framework/Versions/A/ServiceManagement",
        "/System/Library/Frameworks/AppIntents.framework/Versions/A/AppIntents",
    ] {
        reg.insert((*path).into(), HashMap::new());
    }

    reg
}

macro_rules! sym {
    ($map:expr, $name:expr, $fn:expr) => {
        $map.insert($name.into(), $fn as *const () as u64);
    };
}

fn core_foundation_symbols() -> HashMap<String, u64> {
    let mut s = HashMap::new();
    use crate::cf::types::*;
    use crate::cf::string::*;
    use crate::cf::dictionary::*;
    use crate::cf::array::*;
    use crate::cf::data::*;
    use crate::cf::runloop;

    // Type system
    sym!(s, "_CFGetTypeID", CFGetTypeID);
    sym!(s, "_CFRetain", CFRetain);
    sym!(s, "_CFRelease", CFRelease);
    sym!(s, "_CFEqual", CFEqual);
    sym!(s, "_CFHash", CFHash);
    sym!(s, "_CFAllocatorGetDefault", CFAllocatorGetDefault);
    sym!(s, "_CFStringGetTypeID", CFStringGetTypeID);
    sym!(s, "_CFDictionaryGetTypeID", CFDictionaryGetTypeID);
    sym!(s, "_CFArrayGetTypeID", CFArrayGetTypeID);
    sym!(s, "_CFDataGetTypeID", CFDataGetTypeID);
    sym!(s, "_CFBooleanGetTypeID", CFBooleanGetTypeID);
    sym!(s, "_CFNumberGetTypeID", CFNumberGetTypeID);

    // CFString
    sym!(s, "_CFStringCreateWithCString", CFStringCreateWithCString);
    sym!(s, "_CFStringCreateWithBytes", CFStringCreateWithBytes);
    sym!(s, "_CFStringCreateWithFormat", CFStringCreateWithFormat);
    sym!(s, "_CFStringGetLength", CFStringGetLength);
    sym!(s, "_CFStringGetCStringPtr", CFStringGetCStringPtr);
    sym!(s, "_CFStringGetCString", CFStringGetCString);
    sym!(s, "_CFStringGetCharacters", CFStringGetCharacters);
    sym!(s, "_CFStringCompare", CFStringCompare);
    // Constant string class ref
    s.insert("___CFConstantStringClassReference".into(),
        &__CFConstantStringClassReference as *const _ as u64);

    // CFDictionary
    sym!(s, "_CFDictionaryCreate", CFDictionaryCreate);
    sym!(s, "_CFDictionaryCreateMutable", CFDictionaryCreateMutable);
    sym!(s, "_CFDictionaryGetCount", CFDictionaryGetCount);
    sym!(s, "_CFDictionaryGetValue", CFDictionaryGetValue);
    sym!(s, "_CFDictionaryContainsKey", CFDictionaryContainsKey);
    sym!(s, "_CFDictionarySetValue", CFDictionarySetValue);
    sym!(s, "_CFDictionaryRemoveValue", CFDictionaryRemoveValue);
    s.insert("_kCFTypeDictionaryKeyCallBacks".into(),
        &kCFTypeDictionaryKeyCallBacks as *const _ as u64);
    s.insert("_kCFTypeDictionaryValueCallBacks".into(),
        &kCFTypeDictionaryValueCallBacks as *const _ as u64);
    s.insert("_kCFCopyStringDictionaryKeyCallBacks".into(),
        &kCFCopyStringDictionaryKeyCallBacks as *const _ as u64);

    // CFArray
    sym!(s, "_CFArrayCreate", CFArrayCreate);
    sym!(s, "_CFArrayCreateMutable", CFArrayCreateMutable);
    sym!(s, "_CFArrayGetCount", CFArrayGetCount);
    sym!(s, "_CFArrayGetValueAtIndex", CFArrayGetValueAtIndex);
    sym!(s, "_CFArrayAppendValue", CFArrayAppendValue);
    s.insert("_kCFTypeArrayCallBacks".into(),
        &kCFTypeArrayCallBacks as *const _ as u64);

    // CFData
    sym!(s, "_CFDataCreate", CFDataCreate);
    sym!(s, "_CFDataGetLength", CFDataGetLength);
    sym!(s, "_CFDataGetBytePtr", CFDataGetBytePtr);

    // CFRunLoop
    sym!(s, "_CFRunLoopGetCurrent", runloop::CFRunLoopGetCurrent);
    sym!(s, "_CFRunLoopGetMain", runloop::CFRunLoopGetMain);
    sym!(s, "_CFRunLoopRun", runloop::CFRunLoopRun);
    sym!(s, "_CFRunLoopRunInMode", runloop::CFRunLoopRunInMode);
    sym!(s, "_CFRunLoopStop", runloop::CFRunLoopStop);
    sym!(s, "_CFRunLoopWakeUp", runloop::CFRunLoopWakeUp);
    sym!(s, "_CFRunLoopSourceCreate", runloop::CFRunLoopSourceCreate);
    sym!(s, "_CFRunLoopSourceSignal", runloop::CFRunLoopSourceSignal);
    sym!(s, "_CFRunLoopAddSource", runloop::CFRunLoopAddSource);
    sym!(s, "_CFRunLoopRemoveSource", runloop::CFRunLoopRemoveSource);
    sym!(s, "_CFRunLoopTimerCreate", runloop::CFRunLoopTimerCreate);
    sym!(s, "_CFRunLoopAddTimer", runloop::CFRunLoopAddTimer);
    sym!(s, "_CFAbsoluteTimeGetCurrent", runloop::CFAbsoluteTimeGetCurrent);
    s.insert("_kCFRunLoopDefaultMode".into(),
        runloop::kCFRunLoopDefaultMode.as_ptr() as u64);
    s.insert("_kCFRunLoopCommonModes".into(),
        runloop::kCFRunLoopCommonModes.as_ptr() as u64);

    s
}

fn core_graphics_symbols() -> HashMap<String, u64> {
    let mut s = HashMap::new();
    use crate::cg::geometry::*;
    use crate::cg::color::*;
    use crate::cg::context::*;

    // Geometry
    sym!(s, "_CGPointMake", CGPointMake);
    sym!(s, "_CGSizeMake", CGSizeMake);
    sym!(s, "_CGRectMake", CGRectMake);
    sym!(s, "_CGRectGetMinX", CGRectGetMinX);
    sym!(s, "_CGRectGetMinY", CGRectGetMinY);
    sym!(s, "_CGRectGetMaxX", CGRectGetMaxX);
    sym!(s, "_CGRectGetMaxY", CGRectGetMaxY);
    sym!(s, "_CGRectGetMidX", CGRectGetMidX);
    sym!(s, "_CGRectGetMidY", CGRectGetMidY);
    sym!(s, "_CGRectGetWidth", CGRectGetWidth);
    sym!(s, "_CGRectGetHeight", CGRectGetHeight);
    sym!(s, "_CGRectIsEmpty", CGRectIsEmpty);
    sym!(s, "_CGRectIntersection", CGRectIntersection);
    sym!(s, "_CGRectUnion", CGRectUnion);
    sym!(s, "_CGRectContainsPoint", CGRectContainsPoint);
    s.insert("_CGPointZero".into(), &CGPointZero as *const _ as u64);
    s.insert("_CGSizeZero".into(), &CGSizeZero as *const _ as u64);
    s.insert("_CGRectZero".into(), &CGRectZero as *const _ as u64);
    s.insert("_CGRectNull".into(), &CGRectNull as *const _ as u64);
    s.insert("_CGAffineTransformIdentity".into(), &CGAffineTransformIdentity as *const _ as u64);

    // Color
    sym!(s, "_CGColorSpaceCreateDeviceRGB", CGColorSpaceCreateDeviceRGB);
    sym!(s, "_CGColorSpaceCreateDeviceGray", CGColorSpaceCreateDeviceGray);
    sym!(s, "_CGColorSpaceGetNumberOfComponents", CGColorSpaceGetNumberOfComponents);
    sym!(s, "_CGColorSpaceRelease", CGColorSpaceRelease);
    sym!(s, "_CGColorCreate", CGColorCreate);
    sym!(s, "_CGColorGetComponents", CGColorGetComponents);
    sym!(s, "_CGColorRelease", CGColorRelease);

    // Context
    sym!(s, "_CGBitmapContextCreate", CGBitmapContextCreate);
    sym!(s, "_CGBitmapContextGetData", CGBitmapContextGetData);
    sym!(s, "_CGBitmapContextGetWidth", CGBitmapContextGetWidth);
    sym!(s, "_CGBitmapContextGetHeight", CGBitmapContextGetHeight);
    sym!(s, "_CGBitmapContextGetBytesPerRow", CGBitmapContextGetBytesPerRow);
    sym!(s, "_CGContextRelease", CGContextRelease);
    sym!(s, "_CGContextRetain", CGContextRetain);
    sym!(s, "_CGContextSaveGState", CGContextSaveGState);
    sym!(s, "_CGContextRestoreGState", CGContextRestoreGState);
    sym!(s, "_CGContextSetRGBFillColor", CGContextSetRGBFillColor);
    sym!(s, "_CGContextSetRGBStrokeColor", CGContextSetRGBStrokeColor);
    sym!(s, "_CGContextSetGrayFillColor", CGContextSetGrayFillColor);
    sym!(s, "_CGContextFillRect", CGContextFillRect);
    sym!(s, "_CGContextClearRect", CGContextClearRect);
    sym!(s, "_CGContextStrokeRect", CGContextStrokeRect);
    sym!(s, "_CGContextTranslateCTM", CGContextTranslateCTM);
    sym!(s, "_CGContextScaleCTM", CGContextScaleCTM);
    sym!(s, "_CGContextGetCTM", CGContextGetCTM);
    sym!(s, "_CGContextSetLineWidth", CGContextSetLineWidth);

    s
}

fn appkit_symbols() -> HashMap<String, u64> {
    let mut s = HashMap::new();
    use crate::appkit::application;

    // Ensure classes are registered
    crate::appkit::register_classes();

    sym!(s, "_NSApplicationMain", application::NSApplicationMain);

    // NSApplication class methods are dispatched via objc_msgSend,
    // not direct symbol lookup. But some C-level symbols are needed:
    sym!(s, "_NSApp", noop_ptr); // global NSApp pointer — set by sharedApplication

    s
}

fn foundation_symbols() -> HashMap<String, u64> {
    let mut s = HashMap::new();
    // Foundation ObjC classes use objc_msgSend dispatch.
    // We need toll-free bridged C functions:
    // For now, map Foundation CF-bridged symbols to our CF implementations.
    s.extend(core_foundation_symbols());
    s
}

fn carbon_symbols() -> HashMap<String, u64> {
    HashMap::new() // Stub — Carbon is legacy, minimal usage
}

fn quartz_core_symbols() -> HashMap<String, u64> {
    HashMap::new() // Stub — CALayer etc, future work
}

fn core_services_symbols() -> HashMap<String, u64> {
    HashMap::new() // Stub — LaunchServices, FSEvents, etc
}

fn swift_runtime_symbols() -> HashMap<String, u64> {
    let mut s = HashMap::new();
    // Swift runtime entry points — stubs that prevent crashes
    // Real Swift apps need these to initialize the runtime
    sym!(s, "_swift_retain", swift_retain);
    sym!(s, "_swift_release", swift_release);
    sym!(s, "_swift_allocObject", swift_alloc_object);
    sym!(s, "_swift_deallocObject", swift_dealloc_object);
    sym!(s, "_swift_getObjectType", swift_noop_ptr);
    sym!(s, "_swift_conformsToProtocol", swift_noop_false);
    sym!(s, "_swift_dynamicCast", swift_noop_false);
    sym!(s, "_swift_once", swift_once);
    sym!(s, "_swift_beginAccess", swift_noop);
    sym!(s, "_swift_endAccess", swift_noop);
    sym!(s, "$ss27_finalizeUninitializedArrayySayxGABnlF", swift_noop_ptr); // array finalize
    sym!(s, "_swift_bridgeObjectRelease", swift_noop);
    sym!(s, "_swift_bridgeObjectRetain", swift_noop_ptr);
    sym!(s, "_swift_unknownObjectRetain", swift_noop_ptr);
    sym!(s, "_swift_unknownObjectRelease", swift_noop);
    sym!(s, "_swift_deletedMethodError", swift_deleted_method);
    sym!(s, "_swift_getTypeByMangledNameInContext", swift_noop_ptr);
    sym!(s, "_swift_getTypeByMangledNameInContextInMetadataState", swift_noop_ptr);
    sym!(s, "_swift_getExistentialTypeMetadata", swift_noop_ptr);
    sym!(s, "_swift_getGenericMetadata", swift_noop_ptr);
    sym!(s, "_swift_getObjCClassMetadata", swift_noop_ptr);
    sym!(s, "_swift_getWitnessTable", swift_noop_ptr);
    sym!(s, "_swift_getAssociatedTypeWitness", swift_noop_ptr);
    sym!(s, "_swift_checkMetadataState", swift_check_metadata);
    sym!(s, "_swift_getFunctionTypeMetadata", swift_noop_ptr);
    sym!(s, "_swift_getTupleTypeMetadata", swift_noop_ptr);
    sym!(s, "_swift_getMetatypeMetadata", swift_noop_ptr);
    sym!(s, "_swift_allocBox", swift_alloc_box);
    sym!(s, "_swift_projectBox", swift_noop_ptr);
    sym!(s, "_swift_deallocBox", swift_noop);
    sym!(s, "_swift_makeBoxUnique", swift_noop_ptr);
    sym!(s, "_swift_errorRetain", swift_noop_ptr);
    sym!(s, "_swift_errorRelease", swift_noop);
    sym!(s, "_swift_willThrow", swift_noop);
    sym!(s, "_swift_isUniquelyReferenced_nonNull_native", swift_noop_true);
    sym!(s, "_swift_isUniquelyReferenced_native", swift_noop_true);
    sym!(s, "_swift_stdlib_reportFatalError", swift_deleted_method);
    s
}

fn libcxx_symbols() -> HashMap<String, u64> {
    let mut s = HashMap::new();
    // libc++ ABI — minimal stubs
    sym!(s, "___cxa_atexit", swift_noop);
    sym!(s, "___cxa_guard_acquire", cxa_guard_acquire);
    sym!(s, "___cxa_guard_release", swift_noop);
    sym!(s, "___cxa_guard_abort", swift_noop);
    // C++ operator new/delete
    sym!(s, "__Znwm", cxx_new);       // operator new(size_t)
    sym!(s, "__ZdlPv", cxx_delete);    // operator delete(void*)
    sym!(s, "__ZdaPv", cxx_delete);    // operator delete[](void*)
    sym!(s, "__Znam", cxx_new);        // operator new[](size_t)
    s
}

// ---- Stub implementations ----

unsafe extern "C" fn swift_retain(obj: *mut u8) -> *mut u8 { obj }
unsafe extern "C" fn swift_release(_obj: *mut u8) {}
unsafe extern "C" fn swift_alloc_object(_type: *mut u8, size: usize, _align: usize) -> *mut u8 {
    unsafe { libc::calloc(1, size) as *mut u8 }
}
unsafe extern "C" fn swift_dealloc_object(obj: *mut u8, _size: usize, _align: usize) {
    unsafe { libc::free(obj as *mut _) };
}
unsafe extern "C" fn swift_once(predicate: *mut isize, fn_ptr: unsafe extern "C" fn(*mut u8), ctx: *mut u8) {
    // Simple once: if *predicate == 0, call fn and set to -1
    if !predicate.is_null() && unsafe { *predicate } == 0 {
        unsafe { *predicate = -1 };
        unsafe { fn_ptr(ctx) };
    }
}
unsafe extern "C" fn swift_noop() {}
unsafe extern "C" fn swift_noop_ptr(p: *mut u8) -> *mut u8 { p }
unsafe extern "C" fn swift_noop_false() -> i32 { 0 }
unsafe extern "C" fn swift_noop_true() -> i32 { 1 }
unsafe extern "C" fn noop_ptr() -> *mut u8 { std::ptr::null_mut() }
unsafe extern "C" fn swift_deleted_method() {
    let msg = b"grafted: swift_deletedMethodError\n";
    unsafe { libc::write(2, msg.as_ptr() as *const _, msg.len()) };
    unsafe { libc::_exit(1) };
}
unsafe extern "C" fn swift_check_metadata(_req: usize, metadata: *mut u8) -> *mut u8 { metadata }
unsafe extern "C" fn swift_alloc_box(_type: *mut u8) -> *mut u8 {
    unsafe { libc::calloc(1, 64) as *mut u8 }
}
unsafe extern "C" fn cxx_new(size: usize) -> *mut u8 {
    unsafe { libc::malloc(size) as *mut u8 }
}
unsafe extern "C" fn cxx_delete(ptr: *mut u8) {
    if !ptr.is_null() { unsafe { libc::free(ptr as *mut _) }; }
}
unsafe extern "C" fn cxa_guard_acquire(guard: *mut i64) -> i32 {
    if guard.is_null() { return 0; }
    if unsafe { *guard } == 0 { unsafe { *guard = 1 }; 1 } else { 0 }
}

// ObjC runtime globals
#[repr(C)]
pub struct ObjcCache { _mask: u32, _occupied: u32, _buckets: [*mut u8; 1] }
#[unsafe(no_mangle)]
pub static mut __objc_empty_cache: ObjcCache = ObjcCache { _mask: 0, _occupied: 0, _buckets: [std::ptr::null_mut()] };
#[unsafe(no_mangle)]
pub static mut __objc_empty_vtable: [*mut u8; 1] = [std::ptr::null_mut()];
