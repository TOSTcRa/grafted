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
    {
        let mut app_services = core_graphics_symbols();
        app_services.insert("_AXIsProcessTrustedWithOptions".into(), swift_noop_false as *const () as u64);
        reg.insert(
            "/System/Library/Frameworks/ApplicationServices.framework/Versions/A/ApplicationServices".into(),
            app_services,
        );
    }
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
    reg.insert("/usr/lib/libcompression.dylib".into(), HashMap::new());
    reg.insert("/usr/lib/libSystem.B.dylib".into(), system_extras());
    reg.insert("self".into(), system_extras());

    // SwiftUI — minimal implementation to get apps into the event loop
    {
        let mut swiftui = HashMap::new();
        // App.main() → our NSApplicationMain (enters the run loop)
        swiftui.insert("_$s7SwiftUI3AppPAAE4mainyyFZ".into(),
            crate::appkit::application::NSApplicationMain as *const () as u64);
        // App delegate adaptor stubs
        swiftui.insert("_$s7SwiftUI28NSApplicationDelegateAdaptorVMa".into(), swift_noop_ptr as *const () as u64);
        swiftui.insert("_$s7SwiftUI28NSApplicationDelegateAdaptorVMn".into(), swift_noop_ptr as *const () as u64);
        swiftui.insert("_$s7SwiftUI28NSApplicationDelegateAdaptorVyACyxGxmcfC".into(), swift_noop_ptr as *const () as u64);
        reg.insert("/System/Library/Frameworks/SwiftUI.framework/Versions/A/SwiftUI".into(), swiftui);
    }

    // Stub frameworks with empty symbol tables (prevents load errors)
    for path in &[
        // SwiftUI is registered above with actual implementation
        "/System/Library/Frameworks/SwiftData.framework/Versions/A/SwiftData",
        "/System/Library/Frameworks/_SwiftData_SwiftUI.framework/Versions/A/_SwiftData_SwiftUI",
        "/System/Library/Frameworks/Combine.framework/Versions/A/Combine",
        "/System/Library/Frameworks/Vision.framework/Versions/A/Vision",
        "/System/Library/Frameworks/UserNotifications.framework/Versions/A/UserNotifications",
        "/System/Library/Frameworks/ServiceManagement.framework/Versions/A/ServiceManagement",
        "/System/Library/Frameworks/AppIntents.framework/Versions/A/AppIntents",
        "/System/Library/Frameworks/SystemConfiguration.framework/Versions/A/SystemConfiguration",
        "/System/Library/Frameworks/Security.framework/Versions/A/Security",
        "/System/Library/Frameworks/ImageIO.framework/Versions/A/ImageIO",
        "/System/Library/Frameworks/IOKit.framework/Versions/A/IOKit",
        "/System/Library/Frameworks/WebKit.framework/Versions/A/WebKit",
        "/System/Library/Frameworks/UniformTypeIdentifiers.framework/Versions/A/UniformTypeIdentifiers",
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
    // RunLoop observers (stub)
    sym!(s, "_CFRunLoopAddObserver", swift_noop);
    sym!(s, "_CFRunLoopRemoveObserver", swift_noop);
    sym!(s, "_CFRunLoopObserverCreateWithHandler", swift_noop_ptr);
    // NS constants also imported via CoreFoundation
    s.insert("_NSDefaultRunLoopMode".into(), runloop::kCFRunLoopDefaultMode.as_ptr() as u64);
    s.insert("_NSURLFileSizeKey".into(), &NS_CONSTANT_STRINGS as *const _ as u64);

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
    // Extra CG symbols
    sym!(s, "_CGRectInset", cg_rect_inset);
    sym!(s, "_CGColorGetColorSpace", swift_noop_ptr);
    sym!(s, "_CGColorSpaceCopyName", swift_noop_ptr);
    sym!(s, "_CGColorSpaceCreateWithName", CGColorSpaceCreateDeviceRGB);
    sym!(s, "_CGEventCreateKeyboardEvent", swift_noop_ptr);
    sym!(s, "_CGEventPost", swift_noop);
    sym!(s, "_CGEventSetFlags", swift_noop);
    sym!(s, "_CGEventSourceCreate", swift_noop_ptr);
    sym!(s, "_CGEventSourceSetLocalEventsFilterDuringSuppressionState", swift_noop);
    sym!(s, "_CGWindowListCopyWindowInfo", swift_noop_ptr);

    s
}

fn appkit_symbols() -> HashMap<String, u64> {
    let mut s = HashMap::new();
    use crate::appkit::application;
    crate::appkit::register_classes();
    sym!(s, "_NSApplicationMain", application::NSApplicationMain);
    sym!(s, "_NSApp", noop_ptr);
    sym!(s, "_AXIsProcessTrustedWithOptions", swift_noop_false);

    // NS notification/string constants (global CFString-like pointers)
    for name in [
        "_NSAboutPanelOptionCredits",
        "_NSApplicationDidChangeScreenParametersNotification",
        "_NSDefaultRunLoopMode",
        "_NSEventTrackingRunLoopMode",
        "_NSFontAttributeName",
        "_NSForegroundColorAttributeName",
        "_NSKeyValueChangeIndexesKey",
        "_NSKeyValueChangeKindKey",
        "_NSKeyValueChangeNewKey",
        "_NSKeyValueChangeNotificationIsPriorKey",
        "_NSKeyValueChangeOldKey",
        "_NSLinkAttributeName",
        "_NSMenuDidBeginTrackingNotification",
        "_NSMenuDidEndTrackingNotification",
        "_NSPasteboardTypeFileURL",
        "_NSPasteboardTypeHTML",
        "_NSPasteboardTypePDF",
        "_NSPasteboardTypePNG",
        "_NSPasteboardTypeRTF",
        "_NSPasteboardTypeString",
        "_NSPasteboardTypeTIFF",
        "_NSPopoverWillShowNotification",
        "_NSToolbarFlexibleSpaceItemIdentifier",
        "_NSURLFileSizeKey",
        "_NSWindowDidBecomeKeyNotification",
        "_NSWindowDidResignKeyNotification",
    ] {
        s.insert(name.into(), &NS_CONSTANT_STRINGS as *const _ as u64);
    }

    // CG event stubs (used by Maccy for keyboard simulation)
    sym!(s, "_CGEventCreateKeyboardEvent", swift_noop_ptr);
    sym!(s, "_CGEventPost", swift_noop);
    sym!(s, "_CGEventSetFlags", swift_noop);
    sym!(s, "_CGEventSourceCreate", swift_noop_ptr);
    sym!(s, "_CGEventSourceSetLocalEventsFilterDuringSuppressionState", swift_noop);
    sym!(s, "_CGWindowListCopyWindowInfo", swift_noop_ptr);
    sym!(s, "_CGRectInset", cg_rect_inset);
    sym!(s, "_CGColorGetColorSpace", swift_noop_ptr);
    sym!(s, "_CGColorSpaceCopyName", swift_noop_ptr);
    sym!(s, "_CGColorSpaceCreateWithName", crate::cg::color::CGColorSpaceCreateDeviceRGB);

    // QuartzCore
    s.insert("_kCAMediaTimingFunctionEaseInEaseOut".into(), &NS_CONSTANT_STRINGS as *const _ as u64);

    s
}

fn foundation_symbols() -> HashMap<String, u64> {
    let mut s = HashMap::new();
    s.extend(core_foundation_symbols());
    // NS global constant strings
    for name in [
        "_NSDefaultRunLoopMode", "_NSURLFileSizeKey",
        "_NSKeyValueChangeIndexesKey", "_NSKeyValueChangeKindKey",
        "_NSKeyValueChangeNewKey", "_NSKeyValueChangeNotificationIsPriorKey",
        "_NSKeyValueChangeOldKey",
    ] {
        s.insert(name.into(), &NS_CONSTANT_STRINGS as *const _ as u64);
    }
    s
}

fn system_extras() -> HashMap<String, u64> {
    let mut s = HashMap::new();

    // Blocks runtime
    sym!(s, "__Block_copy", block_copy);
    sym!(s, "__Block_release", swift_noop);
    s.insert("__NSConcreteStackBlock".into(), &NS_CONCRETE_STACK_BLOCK as *const _ as u64);
    s.insert("__NSConcreteGlobalBlock".into(), &NS_CONCRETE_STACK_BLOCK as *const _ as u64);

    // GCD dispatch extras
    sym!(s, "_dispatch_once_f", dispatch_once_f);
    sym!(s, "_dispatch_group_create", dispatch_group_create);
    sym!(s, "_dispatch_group_enter", swift_noop);
    sym!(s, "_dispatch_group_leave", swift_noop);

    // dyld
    sym!(s, "__dyld_register_func_for_add_image", swift_noop);
    sym!(s, "_getsectiondata", swift_noop_ptr);

    // os_log
    sym!(s, "__os_log_impl", swift_noop);
    sym!(s, "_os_log_type_enabled", swift_noop_false);
    sym!(s, "_os_release", swift_noop);
    sym!(s, "_voucher_adopt", swift_noop_ptr);

    // libc extras
    sym!(s, "_localtime", libc::localtime);
    sym!(s, "_flockfile", swift_noop);
    sym!(s, "_funlockfile", swift_noop);
    sym!(s, "_malloc_size", malloc_size);

    // Swift stdlib singletons
    s.insert("__swiftEmptyArrayStorage".into(), &SWIFT_EMPTY_ARRAY as *const _ as u64);
    s.insert("__swiftEmptyDictionarySingleton".into(), &SWIFT_EMPTY_DICT as *const _ as u64);
    s.insert("__swiftEmptySetSingleton".into(), &SWIFT_EMPTY_SET as *const _ as u64);
    sym!(s, "__swift_stdlib_operatingSystemVersion", swift_os_version);
    sym!(s, "__swift_stdlib_reportUnimplementedInitializer", swift_deleted_method);

    s
}

fn carbon_symbols() -> HashMap<String, u64> {
    let mut s = HashMap::new();
    // Carbon event stubs
    sym!(s, "_GetEventDispatcherTarget", swift_noop_ptr);
    sym!(s, "_GetEventKind", swift_noop_false);
    sym!(s, "_GetEventParameter", swift_noop_false);
    sym!(s, "_InstallEventHandler", swift_noop_false);
    sym!(s, "_AddEventTypesToHandler", swift_noop_false);
    sym!(s, "_RemoveEventTypesFromHandler", swift_noop_false);
    sym!(s, "_RegisterEventHotKey", swift_noop_false);
    sym!(s, "_UnregisterEventHotKey", swift_noop_false);
    sym!(s, "_CopySymbolicHotKeys", swift_noop_ptr);
    sym!(s, "_LMGetKbdType", swift_noop_false);
    sym!(s, "_UCKeyTranslate", swift_noop_false);
    // Text Input Source
    sym!(s, "_TISCopyCurrentASCIICapableKeyboardInputSource", swift_noop_ptr);
    sym!(s, "_TISCopyCurrentASCIICapableKeyboardLayoutInputSource", swift_noop_ptr);
    sym!(s, "_TISCopyCurrentKeyboardLayoutInputSource", swift_noop_ptr);
    sym!(s, "_TISCreateInputSourceList", swift_noop_ptr);
    sym!(s, "_TISGetInputSourceProperty", swift_noop_ptr);
    // TIS property key constants
    for key in ["_kTISPropertyInputModeID", "_kTISPropertyInputSourceID",
        "_kTISPropertyInputSourceIsASCIICapable", "_kTISPropertyInputSourceIsEnableCapable",
        "_kTISPropertyInputSourceIsEnabled", "_kTISPropertyInputSourceIsSelectCapable",
        "_kTISPropertyInputSourceIsSelected", "_kTISPropertyLocalizedName",
        "_kTISPropertyUnicodeKeyLayoutData",
        "_kTISNotifyEnabledKeyboardInputSourcesChanged",
        "_kTISNotifySelectedKeyboardInputSourceChanged"] {
        s.insert(key.into(), &SWIFT_EMPTY_ARRAY as *const _ as u64); // use as dummy CFString
    }
    s
}

fn quartz_core_symbols() -> HashMap<String, u64> {
    let mut s = HashMap::new();
    s.insert("_kCAMediaTimingFunctionEaseInEaseOut".into(), &NS_CONSTANT_STRINGS as *const _ as u64);
    s
}

fn core_services_symbols() -> HashMap<String, u64> {
    let mut s = HashMap::new();
    sym!(s, "_UCKeyTranslate", swift_noop_false);
    s
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
    sym!(s, "_swift_getInitializedObjCClass", swift_noop_ptr);
    sym!(s, "_swift_lookUpClassMethod", swift_noop_ptr);
    sym!(s, "_swift_initStaticObject", swift_noop_ptr);
    sym!(s, "_swift_initStackObject", swift_noop_ptr);
    sym!(s, "_swift_initClassMetadata2", swift_noop);
    sym!(s, "_swift_initStructMetadata", swift_noop);
    sym!(s, "_swift_updateClassMetadata2", swift_noop);
    sym!(s, "_swift_isaMask", swift_isa_mask_val);
    sym!(s, "_swift_setDeallocating", swift_noop);
    sym!(s, "_swift_deallocClassInstance", swift_dealloc_object);
    sym!(s, "_swift_slowAlloc", swift_slow_alloc);
    sym!(s, "_swift_slowDealloc", swift_slow_dealloc);
    // Retain/release variants
    sym!(s, "_swift_retain_n", swift_noop_ptr);
    sym!(s, "_swift_release_n", swift_noop);
    sym!(s, "_swift_bridgeObjectRelease_n", swift_noop);
    sym!(s, "_swift_bridgeObjectRetain_n", swift_noop_ptr);
    sym!(s, "_swift_unownedRetain", swift_noop_ptr);
    sym!(s, "_swift_unownedRelease", swift_noop);
    sym!(s, "_swift_unownedRetainStrong", swift_noop_ptr);
    // Weak references
    sym!(s, "_swift_weakInit", swift_noop_ptr);
    sym!(s, "_swift_weakAssign", swift_noop_ptr);
    sym!(s, "_swift_weakDestroy", swift_noop);
    sym!(s, "_swift_weakLoadStrong", swift_noop_ptr);
    sym!(s, "_swift_weakTakeInit", swift_noop_ptr);
    sym!(s, "_swift_weakTakeAssign", swift_noop_ptr);
    sym!(s, "_swift_weakCopyInit", swift_noop_ptr);
    sym!(s, "_swift_weakCopyAssign", swift_noop_ptr);
    sym!(s, "_swift_unknownObjectWeakInit", swift_noop_ptr);
    sym!(s, "_swift_unknownObjectWeakAssign", swift_noop_ptr);
    sym!(s, "_swift_unknownObjectWeakDestroy", swift_noop);
    sym!(s, "_swift_unknownObjectWeakLoadStrong", swift_noop_ptr);
    // Dynamic cast
    sym!(s, "_swift_dynamicCastClass", swift_noop_ptr);
    sym!(s, "_swift_dynamicCastMetatype", swift_noop_ptr);
    sym!(s, "_swift_dynamicCastObjCClass", swift_noop_ptr);
    sym!(s, "_swift_dynamicCastObjCProtocolConditional", swift_noop_false);
    sym!(s, "_swift_dynamicCastUnknownClass", swift_noop_ptr);
    sym!(s, "_swift_isEscapingClosureAtFileLocation", swift_noop_false);
    sym!(s, "_swift_isUniquelyReferenced_nonNull_bridgeObject", swift_noop_true);
    sym!(s, "_swift_isUniquelyReferencedNonObjC_nonNull_bridgeObject", swift_noop_true);
    // Metadata
    sym!(s, "_swift_allocateGenericClassMetadata", swift_noop_ptr);
    sym!(s, "_swift_allocateGenericValueMetadata", swift_noop_ptr);
    sym!(s, "_swift_getSingletonMetadata", swift_noop_ptr);
    sym!(s, "_swift_getForeignTypeMetadata", swift_noop_ptr);
    sym!(s, "_swift_getFunctionTypeMetadata0", swift_noop_ptr);
    sym!(s, "_swift_getFunctionTypeMetadata2", swift_noop_ptr);
    sym!(s, "_swift_getTupleTypeMetadata2", swift_noop_ptr);
    sym!(s, "_swift_getTupleTypeMetadata3", swift_noop_ptr);
    sym!(s, "_swift_getObjCClassFromMetadata", swift_noop_ptr);
    sym!(s, "_swift_getOpaqueTypeMetadata", swift_noop_ptr);
    sym!(s, "_swift_getOpaqueTypeMetadata2", swift_noop_ptr);
    sym!(s, "_swift_getOpaqueTypeConformance", swift_noop_ptr);
    sym!(s, "_swift_getOpaqueTypeConformance2", swift_noop_ptr);
    sym!(s, "_swift_getTypeByMangledNameInContext2", swift_noop_ptr);
    sym!(s, "_swift_getTypeByMangledNameInContextInMetadataState2", swift_noop_ptr);
    sym!(s, "_swift_getAssociatedConformanceWitness", swift_noop_ptr);
    // Key paths
    sym!(s, "_swift_getKeyPath", swift_noop_ptr);
    sym!(s, "_swift_getAtKeyPath", swift_noop_ptr);
    // Array operations
    sym!(s, "_swift_arrayDestroy", swift_noop);
    sym!(s, "_swift_arrayInitWithCopy", swift_noop_ptr);
    sym!(s, "_swift_arrayInitWithTakeBackToFront", swift_noop_ptr);
    sym!(s, "_swift_arrayInitWithTakeFrontToBack", swift_noop_ptr);
    // Enum
    sym!(s, "_swift_getEnumCaseMultiPayload", swift_noop_false);
    sym!(s, "_swift_getEnumTagSinglePayloadGeneric", swift_noop_false);
    sym!(s, "_swift_storeEnumTagMultiPayload", swift_noop);
    sym!(s, "_swift_storeEnumTagSinglePayloadGeneric", swift_noop);
    // Error
    sym!(s, "_swift_allocError", swift_noop_ptr);
    sym!(s, "_swift_getErrorValue", swift_noop);
    sym!(s, "_swift_unexpectedError", swift_deleted_method);
    // Misc
    sym!(s, "_swift_stdlib_isStackAllocationSafe", swift_noop_true);
    sym!(s, "_swift_stdlib_random", swift_stdlib_random);
    sym!(s, "_swift_coroFrameAlloc", swift_task_alloc);
    // Swift stdlib
    sym!(s, "__swift_stdlib_operatingSystemVersion", swift_os_version);
    sym!(s, "__swift_stdlib_reportUnimplementedInitializer", swift_deleted_method);
    sym!(s, "__swift_stdlib_isStackAllocationSafe", swift_noop_true);
    sym!(s, "__swift_stdlib_random", swift_stdlib_random);
    s.insert("__swiftEmptyArrayStorage".into(), &SWIFT_EMPTY_ARRAY as *const _ as u64);
    s.insert("__swiftEmptyDictionarySingleton".into(), &SWIFT_EMPTY_DICT as *const _ as u64);
    s.insert("__swiftEmptySetSingleton".into(), &SWIFT_EMPTY_SET as *const _ as u64);
    // Async
    sym!(s, "_swift_async_extendedFramePointerFlags", swift_noop_false);
    // Concurrency
    sym!(s, "_swift_task_create", swift_task_create);
    sym!(s, "_swift_task_alloc", swift_task_alloc);
    sym!(s, "_swift_task_dealloc", swift_noop);
    sym!(s, "_swift_task_switch", swift_noop);
    sym!(s, "_swift_task_enqueue", swift_noop);
    sym!(s, "_swift_task_getCurrent", swift_noop_ptr);
    sym!(s, "_swift_task_getMainExecutor", swift_noop_ptr);
    sym!(s, "_swift_task_isCurrentExecutor", swift_noop_true);
    sym!(s, "_swift_job_run", swift_noop);
    sym!(s, "_swift_asyncLet_begin", swift_noop);
    sym!(s, "_swift_asyncLet_end", swift_noop);
    sym!(s, "_swift_taskGroup_initialize", swift_noop);
    sym!(s, "_swift_taskGroup_destroy", swift_noop);
    sym!(s, "_swift_taskGroup_addPending", swift_noop);
    sym!(s, "_swift_taskGroup_cancelAll", swift_noop);
    sym!(s, "_swift_taskGroup_wait_next_throwing", swift_noop);
    sym!(s, "_swift_continuation_init", swift_noop);
    sym!(s, "_swift_continuation_resume", swift_noop);
    sym!(s, "_swift_continuation_throwingResume", swift_noop);
    sym!(s, "_swift_continuation_throwingResumeWithError", swift_noop);
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
unsafe extern "C" fn swift_task_create(
    _flags: usize, _options: *mut u8, _function: *mut u8,
) -> *mut u8 {
    // Stub: allocate a fake task object
    unsafe { libc::calloc(1, 256) as *mut u8 }
}
unsafe extern "C" fn swift_task_alloc(size: usize) -> *mut u8 {
    unsafe { libc::malloc(size) as *mut u8 }
}
unsafe extern "C" fn swift_isa_mask_val() -> u64 { 0x0000_7FFF_FFFF_FFF8 }
unsafe extern "C" fn swift_slow_alloc(size: usize, align: usize) -> *mut u8 {
    unsafe { libc::memalign(align.max(1), size) as *mut u8 }
}
unsafe extern "C" fn swift_slow_dealloc(ptr: *mut u8, _size: usize, _align: usize) {
    if !ptr.is_null() { unsafe { libc::free(ptr as *mut _) }; }
}
unsafe extern "C" fn swift_stdlib_random(buf: *mut u8, count: usize) {
    // Fill with random bytes via getrandom
    if !buf.is_null() && count > 0 {
        unsafe { libc::getrandom(buf as *mut _, count, 0) };
    }
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

// ---- Global statics ----

// ObjC runtime globals
#[repr(C)]
pub struct ObjcCache { _mask: u32, _occupied: u32, _buckets: [*mut u8; 1] }
#[unsafe(no_mangle)]
pub static mut __objc_empty_cache: ObjcCache = ObjcCache { _mask: 0, _occupied: 0, _buckets: [std::ptr::null_mut()] };
#[unsafe(no_mangle)]
pub static mut __objc_empty_vtable: [*mut u8; 1] = [std::ptr::null_mut()];

// Blocks runtime
#[repr(C)]
struct BlockDescriptor { reserved: u64, size: u64 }
static BLOCK_DESCRIPTOR: BlockDescriptor = BlockDescriptor { reserved: 0, size: 32 };
static NS_CONCRETE_STACK_BLOCK: [u64; 4] = [0; 4];

// Swift stdlib singletons (empty collections)
static SWIFT_EMPTY_ARRAY: [u64; 4] = [0, 1, 0, 0]; // refcount=1, count=0
static SWIFT_EMPTY_DICT: [u64; 8] = [0; 8];
static SWIFT_EMPTY_SET: [u64; 8] = [0; 8];

// NS constant strings (dummy pointers — real apps compare by pointer identity)
static NS_CONSTANT_STRINGS: [u64; 2] = [0; 2];

// ---- Additional stubs ----

unsafe extern "C" fn block_copy(block: *mut u8) -> *mut u8 {
    if block.is_null() { return std::ptr::null_mut(); }
    // Simple: just return the block (stack blocks get "promoted" to heap — we skip that)
    block
}

unsafe extern "C" fn dispatch_once_f(predicate: *mut isize, context: *mut u8, func: unsafe extern "C" fn(*mut u8)) {
    if !predicate.is_null() && unsafe { *predicate } == 0 {
        unsafe { *predicate = -1 };
        unsafe { func(context) };
    }
}

unsafe extern "C" fn dispatch_group_create() -> *mut u8 {
    unsafe { libc::calloc(1, 64) as *mut u8 }
}

unsafe extern "C" fn malloc_size(ptr: *const u8) -> usize {
    if ptr.is_null() { 0 } else { unsafe { libc::malloc_usable_size(ptr as *mut _) } }
}

unsafe extern "C" fn swift_os_version() -> [i64; 3] {
    [14, 0, 0] // Fake macOS 14.0.0 (Sonoma)
}

unsafe extern "C" fn cg_rect_inset(r: crate::cg::geometry::CGRect, dx: f64, dy: f64) -> crate::cg::geometry::CGRect {
    crate::cg::geometry::CGRect {
        origin: crate::cg::geometry::CGPoint {
            x: r.origin.x + dx,
            y: r.origin.y + dy,
        },
        size: crate::cg::geometry::CGSize {
            width: r.size.width - 2.0 * dx,
            height: r.size.height - 2.0 * dy,
        },
    }
}
