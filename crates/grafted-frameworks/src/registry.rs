//! Framework symbol registry - maps Darwin framework paths to our implementations.

use std::collections::HashMap;

/// Returns a map: framework_path -> { symbol_name -> address }.
/// The linker merges these into its symbol registry.
static REAL_GET_TYPE_FN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn framework_registry() -> HashMap<String, HashMap<String, u64>> {
    let mut reg = HashMap::new();

    // Register ObjC classes FIRST so _OBJC_CLASS_$_ fixups find them
    // during chained fixup resolution (before any symbol table build).
    crate::foundation::register_classes();
    crate::appkit::register_classes();

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

    // Swift runtime: try loading real libswiftCore.so from Linux Swift toolchain.
    let real_swift = crate::swift_runtime::swift_symbols();
    if let Some(&addr) = real_swift.get("_swift_getTypeByMangledNameInContext2") {
        log::info!("FOUND REAL SWIFT FN: {:x}", addr);
        REAL_GET_TYPE_FN.store(addr, std::sync::atomic::Ordering::SeqCst);
    } else {
        log::error!("DID NOT FIND REAL SWIFT FN in real_swift map!");
    }
    let swift_syms = if real_swift.is_empty() {
        log::info!("Swift runtime: using stubs (install Swift toolchain for full support)");
        swift_runtime_symbols()
    } else {
        log::info!("Swift runtime: {} real symbols loaded from Linux toolchain", real_swift.len());
        // Merge: real symbols override stubs, except metadata resolution
        // functions that spin on our fake Mach-O metadata
        let stubs_only = swift_runtime_symbols();
        let mut merged = stubs_only.clone();
        merged.extend(real_swift);
        // With the metadata translation layer (swift_metadata_translate.rs),
        if std::env::var("GRAFTED_SWIFT_STUBS").is_ok() {
            log::info!("GRAFTED_SWIFT_STUBS: keeping all metadata stubs");
            for sym_name in [
                "_swift_getSingletonMetadata", "swift_getSingletonMetadata",
                "_swift_conformsToProtocol", "swift_conformsToProtocol",
                "_swift_getWitnessTable", "swift_getWitnessTable",
                "_swift_getAssociatedTypeWitness", "swift_getAssociatedTypeWitness",
                "_swift_getAssociatedConformanceWitness", "swift_getAssociatedConformanceWitness",
                "_swift_checkMetadataState", "swift_checkMetadataState",
                "_swift_getGenericMetadata", "swift_getGenericMetadata",
            ] {
                if let Some(addr) = stubs_only.get(sym_name) {
                    merged.insert(sym_name.into(), *addr);
                }
            }
        } else {
            // Only swift_conformsToProtocol stays as stub - it scans ALL
            for sym_name in [
                "_swift_conformsToProtocol", "swift_conformsToProtocol",
                "_swift_getWitnessTable", "swift_getWitnessTable",
                "_swift_getGenericMetadata", "swift_getGenericMetadata",
                "_swift_getTypeByMangledNameInContext2", "swift_getTypeByMangledNameInContext2",
                "_swift_allocateWitnessTablePack", "swift_allocateWitnessTablePack",
                "_swift_deallocateWitnessTablePack", "swift_deallocateWitnessTablePack",
                // These functions have internal call paths within libswiftCore.so
                // that bypass our entry-point JMP patches. Keep as stubs permanently.
                "_swift_getKeyPath", "swift_getKeyPath",
                "_swift_getGenericMetadata", "swift_getGenericMetadata",
                "_swift_getSingletonMetadata", "swift_getSingletonMetadata",
            ] {
                if let Some(addr) = stubs_only.get(sym_name) {
                    merged.insert(sym_name.into(), *addr);
                }
            }
            log::info!("Swift runtime: real metadata functions (1 stub: conformsToProtocol)");
        }
        merged
    };

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
        reg.insert((*lib).into(), swift_syms.clone());
    }

    // Darwin frameworks also import Swift symbols (e.g. Foundation.Notification).
    // Merge swift_syms into framework entries so they resolve correctly.
    for fw in &[
        "/System/Library/Frameworks/Foundation.framework/Versions/C/Foundation",
        "/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit",
        "/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation",
        "/System/Library/Frameworks/CoreGraphics.framework/Versions/A/CoreGraphics",
        "/System/Library/Frameworks/CoreServices.framework/Versions/A/CoreServices",
    ] {
        if let Some(entry) = reg.get_mut::<str>(fw) {
            // Swift symbols fill gaps - don't overwrite framework-specific stubs
            for (k, v) in &swift_syms {
                entry.entry(k.clone()).or_insert(*v);
            }
        }
    }

    // Other system libraries
    reg.insert("/usr/lib/libc++.1.dylib".into(), libcxx_symbols());
    reg.insert("/usr/lib/libc++abi.dylib".into(), libcxx_symbols());
    reg.insert("/usr/lib/libcompression.dylib".into(), HashMap::new());
    reg.insert("/usr/lib/libSystem.B.dylib".into(), system_extras());
    reg.insert("self".into(), system_extras());

    // SwiftUI - use our compiled shim (shims/libSwiftUI.so) symbols if available,
    // otherwise fall back to stubs.
    {
        let mut swiftui = HashMap::new();
        // Start with stub defaults
        swiftui.insert("_$s7SwiftUI3AppPAAE4mainyyFZ".into(),
            crate::appkit::application::NSApplicationMain as *const () as u64);
        swiftui.insert("_$s7SwiftUI28NSApplicationDelegateAdaptorVMa".into(), swift_metadata_accessor_stub as *const () as u64);
        swiftui.insert("_$s7SwiftUI28NSApplicationDelegateAdaptorVMn".into(), swift_metadata_accessor_stub as *const () as u64);
        swiftui.insert("_$s7SwiftUI28NSApplicationDelegateAdaptorVyACyxGxmcfC".into(), swift_noop_ptr as *const () as u64);
        // SwiftUI core type metadata accessors - fix for RAX=0x0 crash at 0x10006fcd1
        swiftui.insert("_$s7SwiftUI18LocalizedStringKeyVMa".into(), swift_metadata_accessor_stub as *const () as u64);
        swiftui.insert("_$s7SwiftUI18LocalizedStringKeyVMn".into(), swift_metadata_accessor_stub as *const () as u64);
        swiftui.insert("_$s7SwiftUI4TextVMa".into(), swift_metadata_accessor_stub as *const () as u64);
        swiftui.insert("_$s7SwiftUI4TextVMn".into(), swift_metadata_accessor_stub as *const () as u64);
        swiftui.insert("_$s7SwiftUI4ViewPMa".into(), swift_metadata_accessor_stub as *const () as u64);
        swiftui.insert("_$s7SwiftUI4ViewPMn".into(), swift_metadata_accessor_stub as *const () as u64);
        // Override with REAL shim symbols (from compiled libSwiftUI.so)
        // These include App.main() that calls body getter -> creates real windows
        for (k, v) in &swift_syms {
            if k.contains("SwiftUI") {
                // Intercept symbols that require String ABI translation
                if k == "$s7SwiftUI18LocalizedStringKeyV13stringLiteralACSS_tcfC" ||
                   k == "_$s7SwiftUI18LocalizedStringKeyV13stringLiteralACSS_tcfC" {
                    log::info!("Swift ABI: Intercepting LocalizedStringKey init");
                    swiftui.insert(k.clone(), crate::swift_runtime::bridge_SwiftUI_LocalizedStringKey_init as *const () as u64);
                } else {
                    swiftui.insert(k.clone(), *v);
                }
            }
        }
        let target_sym = "_$s7SwiftUI12MenuBarExtraVA2A4TextVRszrlE_10isInserted7contentACyAEq_GAA18LocalizedStringKeyV_AA7BindingVySbGq_yXEtcfC";
        let in_swift_syms = swift_syms.contains_key(target_sym);
        let in_swiftui = swiftui.contains_key(target_sym);
        // Also check partial match
        let partial = swift_syms.keys().filter(|k| k.contains("MenuBarExtra") && k.contains("isInserted")).count();
        for k in swift_syms.keys() {
            if k.contains("MenuBarExtra") && k.contains("isInserted") {
                log::info!("  FOUND: {}", k);
            }
        }
        log::info!("  NEED:  {}", target_sym);
        log::info!("MenuBarExtra.init: in_swift_syms={} in_swiftui={} partial={} total={}",
            in_swift_syms, in_swiftui, partial, swiftui.len());
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
    // Nested-mode constants (Step 1). Point _NSEventTrackingRunLoopMode /
    s.insert("_NSEventTrackingRunLoopMode".into(),
        runloop::NSEventTrackingRunLoopMode.as_ptr() as u64);
    s.insert("_NSModalPanelRunLoopMode".into(),
        runloop::NSModalPanelRunLoopMode.as_ptr() as u64);
    s.insert("_UITrackingRunLoopMode".into(),
        runloop::UITrackingRunLoopMode.as_ptr() as u64);
    // Observer functions (real implementations; Step 1 replaced the previous stubs)
    sym!(s, "_CFRunLoopObserverCreate", runloop::CFRunLoopObserverCreate);
    sym!(s, "_CFRunLoopAddObserver", runloop::CFRunLoopAddObserver);
    sym!(s, "_CFRunLoopRemoveObserver", runloop::CFRunLoopRemoveObserver);
    sym!(s, "_CFRunLoopContainsSource", runloop::CFRunLoopContainsSource);
    sym!(s, "_CFRunLoopRemoveTimer", runloop::CFRunLoopRemoveTimer);
    sym!(s, "_CFRunLoopAddCommonMode", runloop::CFRunLoopAddCommonMode);
    sym!(s, "_CFRunLoopCopyCurrentMode", runloop::CFRunLoopCopyCurrentMode);
    // Handler-block variant is unimplemented (requires block support)
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

    // ObjC class pointers for _OBJC_CLASS_$_ imports that come through CoreFoundation.
    // register_classes() was already called at the top of framework_registry().
    for class_name in &[
        "NSUserDefaults", "NSString", "NSMutableString", "NSNumber", "NSNull",
        "NSBundle", "NSNotificationCenter", "NSDistributedNotificationCenter",
        "NSProcessInfo", "NSFileManager", "NSObject", "NSArray", "NSMutableArray",
        "NSDictionary", "NSMutableDictionary", "NSData", "NSMutableData",
        "NSURL", "NSSet", "NSMutableSet", "NSDate", "NSError",
        "NSNumberFormatter", "NSByteCountFormatter",
        "NSMutableAttributedString", "NSAttributedString",
    ] {
        let c_name = std::ffi::CString::new(*class_name).unwrap();
        let cls = grafted_objc::objc_getClass(c_name.as_ptr());
        if !cls.is_null() {
            s.insert(format!("_OBJC_CLASS_$_{}", class_name), cls as u64);
            s.insert(format!("_OBJC_METACLASS_$_{}", class_name), cls as u64);
        }
    }

    s
}

fn appkit_symbols() -> HashMap<String, u64> {
    let mut s = HashMap::new();
    use crate::appkit::application;
    crate::foundation::register_classes();
    crate::appkit::register_classes();
    sym!(s, "_NSApplicationMain", application::NSApplicationMain);
    sym!(s, "_NSApp", noop_ptr);
    sym!(s, "_AXIsProcessTrustedWithOptions", swift_noop_false);

    // NS notification/string constants (global CFString-like pointers)
    for name in [
        "_NSAboutPanelOptionCredits",
        "_NSApplicationDidChangeScreenParametersNotification",
        // _NSDefaultRunLoopMode / _NSEventTrackingRunLoopMode / _NSModalPanelRunLoopMode
        // registered above with real runloop mode pointers (see Step 1).
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
        // _NSDefaultRunLoopMode is bound to the real runloop default-mode pointer
        // inside core_foundation_symbols(); do NOT overwrite it here.
        "_NSURLFileSizeKey",
        "_NSKeyValueChangeIndexesKey", "_NSKeyValueChangeKindKey",
        "_NSKeyValueChangeNewKey", "_NSKeyValueChangeNotificationIsPriorKey",
        "_NSKeyValueChangeOldKey",
    ] {
        s.insert(name.into(), &NS_CONSTANT_STRINGS as *const _ as u64);
    }
    // Expose registered Foundation class pointers for _OBJC_CLASS_$_ imports.
    // register_classes() was already called at the top of framework_registry().
    for class_name in &[
        "NSUserDefaults", "NSString", "NSMutableString", "NSNumber", "NSNull",
        "NSBundle", "NSNotificationCenter", "NSDistributedNotificationCenter",
        "NSProcessInfo", "NSFileManager", "NSNumberFormatter",
        "NSByteCountFormatter", "NSMutableAttributedString", "NSAttributedString",
    ] {
        let c_name = std::ffi::CString::new(*class_name).unwrap();
        let cls = grafted_objc::objc_getClass(c_name.as_ptr());
        if !cls.is_null() {
            s.insert(format!("_OBJC_CLASS_$_{}", class_name), cls as u64);
            s.insert(format!("_OBJC_METACLASS_$_{}", class_name), cls as u64);
        }
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
    // Swift runtime entry points - stubs that prevent crashes
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
    sym!(s, "_swift_getTypeByMangledNameInContext", swift_get_type_by_mangled_name);
    sym!(s, "_swift_getTypeByMangledNameInContextInMetadataState", swift_get_type_by_mangled_name);
    sym!(s, "_swift_getExistentialTypeMetadata", swift_noop_ptr);
    sym!(s, "_swift_getGenericMetadata", swift_get_generic_metadata_stub);
    sym!(s, "_swift_getObjCClassMetadata", swift_noop_ptr);
    sym!(s, "_swift_getWitnessTable", smart_getWitnessTable);
    sym!(s, "_swift_getAssociatedTypeWitness", swift_noop_ptr);
    sym!(s, "_swift_checkMetadataState", swift_check_metadata);
    sym!(s, "_swift_getFunctionTypeMetadata", swift_noop_ptr);
    sym!(s, "_swift_getTupleTypeMetadata", swift_noop_ptr);
    sym!(s, "_swift_allocateWitnessTablePack", swift_noop_ptr);
    sym!(s, "_swift_deallocateWitnessTablePack", swift_noop);
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
    sym!(s, "_swift_getSingletonMetadata", swift_get_singleton_metadata);
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
    sym!(s, "_swift_getTypeByMangledNameInContext2", swift_get_type_by_mangled_name);
    sym!(s, "_swift_getTypeByMangledNameInContextInMetadataState2", swift_get_type_by_mangled_name);
    sym!(s, "_swift_getAssociatedConformanceWitness", swift_noop_ptr);
    // Key paths
    sym!(s, "_swift_getKeyPath", safe_return_stub_object);
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
    // libc++ ABI - minimal stubs
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

// swift_retain/release are defined below with safe interposition logic
unsafe extern "C" fn swift_alloc_object(metadata: *mut u8, size: usize, _align: usize) -> *mut u8 {
    let obj = unsafe { libc::calloc(1, size.max(16)) } as *mut u8;
    if !obj.is_null() {
        unsafe {
            // HeapObject layout: +0 = metadata, +8 = InlineRefCounts
            *(obj as *mut *mut u8) = metadata;
            // Set refcount to "1 strong reference" (immortal-ish: high bits set
            // so retain/release don't underflow to dealloc)
            *((obj as *mut u64).add(1)) = 0xFFFFFFFFFFFFFFFF; // immortal refcount: (rc & 0x80000000FFFFFFFF) == mask
        }
    }
    obj
}
unsafe extern "C" fn swift_dealloc_object(obj: *mut u8, _size: usize, _align: usize) {
    unsafe { libc::free(obj as *mut _) };
}
unsafe extern "C" fn swift_once(predicate: *mut isize, fn_ptr: unsafe extern "C" fn(*mut u8), ctx: *mut u8) {
    if !predicate.is_null() && unsafe { *predicate } == 0 {
        unsafe { *predicate = -1 };
        unsafe { fn_ptr(ctx) };
    }
}
unsafe extern "C" fn swift_noop() {}
unsafe extern "C" fn swift_noop_ptr(p: *mut u8) -> *mut u8 { p }

/// Proper metadata accessor stub that returns valid metadata pointer instead of MetadataRequest
unsafe extern "C" fn swift_metadata_accessor_stub(_request: u64) -> *const u8 {
    // Create a minimal valid metadata structure
    // Swift metadata starts with a kind field (u64) and value witness table pointer
    static mut STUB_METADATA: [u64; 16] = [0; 16];
    static INIT_ONCE: std::sync::Once = std::sync::Once::new();

    INIT_ONCE.call_once(|| unsafe {
        // Set metadata kind to Class (1)
        STUB_METADATA[0] = 1;
        // Set VWT pointer to a proper VWT stub (not null!)
        STUB_METADATA[1] = safe_vwt_ptr() as u64;
        // Initialize other fields to safe defaults
        for i in 2..16 {
            STUB_METADATA[i] = 0;
        }
    });

    unsafe { std::ptr::addr_of!(STUB_METADATA) as *const u8 }
}

unsafe extern "C" fn swift_noop_false() -> i32 { 0 }
unsafe extern "C" fn swift_noop_true() -> i32 { 1 }
unsafe extern "C" fn noop_ptr() -> *mut u8 { std::ptr::null_mut() }

// Simple static dummy value (non-null address)
static DUMMY_VWT: u64 = 0x1000;

unsafe extern "C" fn safe_vwt_ptr() -> *const u8 {
    &DUMMY_VWT as *const u64 as *const u8
}
unsafe extern "C" fn swift_deleted_method() {
    let msg = b"grafted: swift_deletedMethodError\n";
    unsafe { libc::write(2, msg.as_ptr() as *const _, msg.len()) };
    unsafe { libc::_exit(1) };
}
unsafe extern "C" fn swift_check_metadata(metadata: *mut u8, _request: usize) -> MetadataResponse {
    // During lifecycle: ALWAYS return our stub metadata (with valid VWT, size=8).
    if LIFECYCLE_PATCHES_ACTIVE.load(std::sync::atomic::Ordering::Relaxed) {
        return MetadataResponse { metadata: shim_unresolved_soft_metadata(), state: 0 };
    }
    // Outside lifecycle: return the input metadata with Complete state.
    let addr = metadata as usize;
    if addr <= 0x1000 || addr >= 0x800000000000 {
        MetadataResponse { metadata: shim_unresolved_soft_metadata(), state: 0 }
    } else {
        MetadataResponse { metadata, state: 0 }
    }
}

/// MetadataResponse: returned in (rax=metadata, rdx=state) on x86_64
#[repr(C)]
pub struct MetadataResponse {
    pub metadata: *mut u8,
    pub state: usize,
}

/// Our replacement for swift_getAssociatedTypeWitness.
/// During lifecycle: returns stub metadata. Otherwise: calls real via trampoline.
static REAL_GET_ASSOC_TYPE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

unsafe extern "C" fn safe_getAssociatedTypeWitness(
    request: usize,
    witness_table: *const u8,
    conforming_type: *const u8,
    req_base: *const u8,
    assoc_type: *const u8,
) -> MetadataResponse {
    if LIFECYCLE_PATCHES_ACTIVE.load(std::sync::atomic::Ordering::Relaxed) {
        let stub = shim_unresolved_soft_metadata();
        return MetadataResponse { metadata: stub, state: 0 };
    }
    let trampoline = REAL_GET_ASSOC_TYPE.load(std::sync::atomic::Ordering::Acquire);
    if trampoline != 0 {
        type F = unsafe extern "C" fn(usize, *const u8, *const u8, *const u8, *const u8) -> MetadataResponse;
        let f: F = std::mem::transmute(trampoline);
        return f(request, witness_table, conforming_type, req_base, assoc_type);
    }
    MetadataResponse { metadata: shim_unresolved_soft_metadata(), state: 0 }
}

/// No-op witness function: returns stub object with String-safe fields.
unsafe extern "C" fn witness_noop(_obj: *mut u8, _wt: *mut u8) -> *mut u8 {
    safe_return_stub_object(std::ptr::null_mut(), std::ptr::null_mut())
}

/// Get a valid stub witness table filled with no-op function pointers.
fn get_stub_witness_table() -> *mut u8 {
    static STUB_WT: std::sync::atomic::AtomicPtr<u8> = std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());
    let wt = STUB_WT.load(std::sync::atomic::Ordering::Acquire);
    if !wt.is_null() { return wt; }

    let p = unsafe { libc::calloc(1, 512) } as *mut u64;
    let func = witness_noop as *const () as u64;
    // Fill ALL entries with the no-op function pointer
    unsafe { for i in 0..64 { *p.add(i) = func; } }
    let wt = p as *mut u8;
    STUB_WT.store(wt, std::sync::atomic::Ordering::Release);
    wt
}

/// No-op function for patching init functions that corrupt our stub metadata.
unsafe extern "C" fn safe_noop_return() {
    unsafe {
        std::arch::asm!(
            "xor eax, eax",
            "mov rdx, 0xE000000000000000",
            out("rax") _,
            out("rdx") _,
            options(nomem, nostack)
        );
    }
}
/// Return a stub that works BOTH as metadata (VWT at [-1]) AND as a
/// heap object (String-safe fields at positive offsets).
unsafe extern "C" fn safe_return_stub_object(_a: *mut u8, _b: *mut u8) -> *mut u8 {
    static STUB_OBJ: std::sync::atomic::AtomicPtr<u8> = std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());
    let obj = STUB_OBJ.load(std::sync::atomic::Ordering::Acquire);
    if !obj.is_null() { return obj; }

    // Allocate with space before for VWT pointer (like metadata layout)
    let raw = unsafe { libc::calloc(1, 1024) } as *mut u64;
    let stub_meta = shim_unresolved_soft_metadata();
    let good_vwt = unsafe { *((stub_meta as *const u64).sub(1)) };

    // obj starts 8 bytes in, so obj[-1] is raw[0] (VWT pointer)
    let obj_ptr = unsafe { raw.add(1) };
    unsafe {
        *raw = good_vwt;                           // obj[-1] = valid VWT
        *obj_ptr = 0x200;                          // obj[0] = kind (metadata compat)
        *obj_ptr.add(1) = stub_meta as u64;        // obj[1] = descriptor
        // Fill remaining with empty String patterns + self-references
        for i in (2..120).step_by(2) {
            *obj_ptr.add(i) = 0;                   // String word0 = 0
            *obj_ptr.add(i + 1) = 0xE000000000000000; // String word1 = empty small
        }
    }
    let obj = obj_ptr as *mut u8;
    STUB_OBJ.store(obj, std::sync::atomic::Ordering::Release);
    // Set rdx to empty-String discriminator for callers expecting 16-byte returns
    unsafe { std::arch::asm!("mov rdx, 0xE000000000000000", out("rdx") _, options(nomem, nostack, preserves_flags)); }
    obj
}
/// Return stub metadata (not null - null metadata causes VWT[-1] crashes)
unsafe extern "C" fn safe_return_null(_a: *mut u8, _b: *mut u8) -> *mut u8 {
    unsafe { std::arch::asm!("mov rdx, 0xE000000000000000", out("rdx") _, options(nomem, nostack, preserves_flags)); }
    shim_unresolved_soft_metadata()
}
/// Return false (0)
unsafe extern "C" fn safe_return_false(_a: *mut u8, _b: *mut u8) -> i32 {
    unsafe { std::arch::asm!("mov rdx, 0xE000000000000000", out("rdx") _, options(nomem, nostack, preserves_flags)); }
    0
}
unsafe extern "C" fn safe_return_stub_metadata(_a: *mut u8, _b: usize, _c: *mut u8, _d: *mut u8) -> *mut u8 {
    unsafe { std::arch::asm!("mov rdx, 0xE000000000000000", out("rdx") _, options(nomem, nostack, preserves_flags)); }
    shim_unresolved_soft_metadata()
}

/// swift_once for lifecycle: try the callback, skip if it would crash.
/// Uses a secondary setjmp/longjmp to catch crashes within the callback.
unsafe extern "C" fn swift_once_lifecycle(predicate: *mut isize, fn_ptr: unsafe extern "C" fn(*mut u8), ctx: *mut u8) {
    if !predicate.is_null() && unsafe { *predicate } == 0 {
        unsafe { *predicate = -1 };
        // Try calling the callback - if it crashes, skip it
        unsafe extern "C" { fn grafted_try_call(f: unsafe extern "C" fn(*mut u8, *mut u8), a: *mut u8, b: *mut u8) -> bool; }
        // Wrap the single-arg fn_ptr as a two-arg call (second arg unused)
        type TwoArgFn = unsafe extern "C" fn(*mut u8, *mut u8);
        let f: TwoArgFn = std::mem::transmute(fn_ptr as *const ());
        let ok = unsafe { grafted_try_call(f, ctx, std::ptr::null_mut()) };
        if !ok {
            log::debug!("swift_once callback crashed - skipped");
        }
    }
}

/// Safe swift_conformsToProtocol: returns stub witness table instead of NULL.
unsafe extern "C" fn safe_conformsToProtocol(
    _type: *const u8,
    _protocol: *const u8,
) -> *const u8 {
    get_stub_witness_table() as *const u8
}

/// Our replacement for swift_getAssociatedConformanceWitness.
unsafe extern "C" fn safe_getAssociatedConformanceWitness(
    _witness_table: *const u8,
    _conforming_type: *const u8,
    _assoc_type: *const u8,
    _req_base: *const u8,
    _assoc_conformance: *const u8,
) -> *const u8 {
    swift_get_witness_table_stub(std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) as *const u8
}

/// Patch ALL loaded instances of a function + its local Slow variant.
/// Uses dl_iterate_phdr to find every loaded libswiftCore.so and patches each one.
unsafe fn patch_all_instances(exported_name: &str, slow_offset: usize, replacement: *const u8) {
    use std::ffi::CString;

    // First, find the offset of the exported function within libswiftCore.so
    // by comparing dlsym result with the library's load address.
    let c_name = CString::new(exported_name).unwrap();

    // Collect all libswiftCore.so base addresses via /proc/self/maps
    let mut bases: Vec<usize> = Vec::new();
    if let Ok(maps) = std::fs::read_to_string("/proc/self/maps") {
        for line in maps.lines() {
            if line.contains("libswiftCore.so") && line.contains("r-xp") {
                // Parse the start address from "55a1234000-55a1240000 r-xp ..."
                if let Some(addr_str) = line.split('-').next() {
                    if let Ok(addr) = usize::from_str_radix(addr_str.trim(), 16) {
                        bases.push(addr);
                    }
                }
            }
        }
    }

    if bases.is_empty() { return; }

    // Find the exported function's offset from the first base
    let sym_addr = libc::dlsym(libc::RTLD_DEFAULT, c_name.as_ptr()) as usize;
    if sym_addr == 0 { return; }

    // Determine which base this symbol belongs to
    let mut sym_offset = 0usize;
    for &base in &bases {
        if sym_addr >= base && sym_addr < base + 0x1000000 {
            sym_offset = sym_addr - base;
            break;
        }
    }
    if sym_offset == 0 { return; }

    // Now patch the Slow variant in ALL loaded copies
    for &base in &bases {
        let slow_addr = (base + sym_offset + slow_offset) as *mut u8;
        // Verify the target looks like a function (starts with push %rbp or similar)
        let first_byte = std::ptr::read_volatile(slow_addr);
        if first_byte == 0x55 || first_byte == 0x48 || first_byte == 0x41 {
            log::info!("Patching {}Slow at {:#x} (base {:#x})", exported_name, slow_addr as usize, base);
            patch_function_at(slow_addr, replacement);
        }
    }
}

/// Patch a function at a raw address with a JMP to our replacement.
unsafe fn patch_function_at(target: *mut u8, replacement: *const u8) {
    if target.is_null() { return; }

    let page = (target as usize & !0xFFF) as *mut libc::c_void;
    if libc::mprotect(page, 8192, libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC) != 0 {
        return;
    }

    let p = target;
    *p = 0x48;
    *p.add(1) = 0xB8;
    std::ptr::write_unaligned(p.add(2) as *mut u64, replacement as u64);
    *p.add(10) = 0xFF;
    *p.add(11) = 0xE0;

    libc::mprotect(page, 8192, libc::PROT_READ | libc::PROT_EXEC);
}

/// Patch a function in a loaded library by overwriting its first bytes with
/// a JMP to our replacement. This works for internal calls that don't go through PLT.
unsafe fn patch_function(name: &str, replacement: *const u8) {
    let c_name = std::ffi::CString::new(name).unwrap();
    let target = libc::dlsym(libc::RTLD_DEFAULT, c_name.as_ptr());
    if target.is_null() { return; }

    // Make the page writable
    let page = (target as usize & !0xFFF) as *mut libc::c_void;
    if libc::mprotect(page, 4096, libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC) != 0 {
        return;
    }

    // Write: mov rax, <address>; jmp rax  (12 bytes)
    let p = target as *mut u8;
    *p = 0x48;                               // REX.W
    *p.add(1) = 0xB8;                        // mov rax, imm64
    std::ptr::write_unaligned(p.add(2) as *mut u64, replacement as u64);
    *p.add(10) = 0xFF;                       // jmp rax
    *p.add(11) = 0xE0;

    // Restore page protection
    libc::mprotect(page, 4096, libc::PROT_READ | libc::PROT_EXEC);
    log::info!("Patched {} -> {:#x}", name, replacement as u64);
}

/// Get/create the universal stub metadata pointer (with VWT, descriptor, etc.)
fn shim_unresolved_soft_metadata() -> *mut u8 {
    static STUB: std::sync::atomic::AtomicPtr<u8> = std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());
    let ptr = STUB.load(std::sync::atomic::Ordering::Acquire);
    if !ptr.is_null() { return ptr; }

    // Create metadata with proper VWT
    let raw = unsafe { libc::calloc(1, 512) } as *mut u64;
    let metadata = unsafe { raw.add(8) };
    let vwt = unsafe { libc::calloc(1, 256) } as *mut u64;
    unsafe extern "C" fn dummy_fn(arg: *mut u8) -> *mut u8 { arg }
    let d = dummy_fn as *const () as u64;
    unsafe {
        for i in 0..8 { *vwt.add(i) = d; }
        *vwt.add(8) = 8; *vwt.add(9) = 8; *vwt.add(10) = 7;
        let desc = libc::calloc(1, 256) as *mut u64;
        *metadata.sub(1) = vwt as u64;
        *metadata = 0x200;
        *metadata.add(1) = desc as u64;
        for i in 2..48 { *metadata.add(i) = metadata as u64; }
    }
    let p = metadata as *mut u8;
    STUB.store(p, std::sync::atomic::Ordering::Release);
    p
}

static REAL_CHECK_METADATA: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Flag: when true, patches are active (during applicationDidFinishLaunching).
/// When false, patches call through to the real implementation.
pub static LIFECYCLE_PATCHES_ACTIVE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Smart swift_checkMetadataState: during lifecycle calls, returns Complete
/// for stub metadata. Otherwise calls the real implementation via trampoline.
unsafe extern "C" fn smart_checkMetadataState(metadata: *mut u8, request: usize) -> MetadataResponse {
    let addr = metadata as usize;

    // During lifecycle: ALWAYS return Complete. Never call the real function -
    // it triggers recursive metadata resolution that stack-overflows.
    if LIFECYCLE_PATCHES_ACTIVE.load(std::sync::atomic::Ordering::Relaxed) {
        if addr <= 0x1000 || addr >= 0x800000000000 {
            return MetadataResponse { metadata: shim_unresolved_soft_metadata(), state: 0 };
        }
        return MetadataResponse { metadata, state: 0 };
    }

    // Outside lifecycle: for invalid pointers return stub, otherwise call real
    if addr <= 0x1000 || addr >= 0x800000000000 {
        return MetadataResponse { metadata: shim_unresolved_soft_metadata(), state: 0 };
    }

    let trampoline = REAL_CHECK_METADATA.load(std::sync::atomic::Ordering::Acquire);
    if trampoline != 0 {
        type RealFn = unsafe extern "C" fn(*mut u8, usize) -> MetadataResponse;
        let f: RealFn = std::mem::transmute(trampoline);
        let resp = f(metadata, request);
        // Validate VWT after the real function - swift_initStructMetadata may
        // have corrupted metadata[-1] with computed garbage (e.g., 0x211).
        if !resp.metadata.is_null() && (resp.metadata as usize) > 0x1000 {
            let vwt = *((resp.metadata as *const u64).sub(1));
            if vwt < 0x10000 {
                // VWT is garbage - replace with our universal VWT
                let stub = shim_unresolved_soft_metadata();
                let good_vwt = *((stub as *const u64).sub(1));
                *((resp.metadata as *mut u64).sub(1)) = good_vwt;
            }
        }
        return resp;
    }
    MetadataResponse { metadata, state: 0 }
}

// swift_checkMetadataState is now handled via temporary binary patching
// in apply_lifecycle_patches/restore_lifecycle_patches.
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
/// swift_getSingletonMetadata(request, descriptor) -> (metadata, state)
unsafe extern "C" fn swift_get_singleton_metadata(
    _request: usize,
    descriptor: *const u8,
) -> *mut u8 {
    use std::collections::HashMap;
    use std::sync::Mutex;
    struct Cache(HashMap<usize, *mut u8>);
    unsafe impl Send for Cache {}
    static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

    let key = descriptor as usize;
    let mut cache = CACHE.lock().unwrap();
    let map = &mut cache.get_or_insert_with(|| Cache(HashMap::new())).0;

    if let Some(&ptr) = map.get(&key) {
        return ptr;
    }

    // Allocate a realistic metadata struct with value witness table.

    // First allocate a minimal VWT
    let vwt = unsafe { libc::calloc(1, 256) } as *mut u64;
    unsafe {
        // VWT layout (see swift/ABI/ValueWitness.def):

        // VWT functions with correct Swift ABI signatures
        unsafe extern "C" fn vwt_initcopy(dest: *mut u8, src: *mut u8, _meta: *const u8) -> *mut u8 {
            std::ptr::copy_nonoverlapping(src, dest, 8);
            dest
        }
        unsafe extern "C" fn vwt_destroy(_: *mut u8, _: *const u8) {}
        unsafe extern "C" fn vwt_enum_tag(_: *const u8, _: u32, _: *const u8) -> u32 { 0 }
        unsafe extern "C" fn vwt_store_enum_tag(_: *mut u8, _: u32, _: u32, _: *const u8) {}

        // Populate VWT slots 0-7 with correct function pointers
        *vwt.add(0) = vwt_initcopy as *const () as u64;  // initializeBufferWithCopyOfBuffer
        *vwt.add(1) = vwt_destroy as *const () as u64;   // destroy
        *vwt.add(2) = vwt_initcopy as *const () as u64;  // initializeWithCopy
        *vwt.add(3) = vwt_initcopy as *const () as u64;  // assignWithCopy
        *vwt.add(4) = vwt_initcopy as *const () as u64;  // initializeWithTake
        *vwt.add(5) = vwt_initcopy as *const () as u64;  // assignWithTake
        *vwt.add(6) = vwt_enum_tag as *const () as u64;  // getEnumTagSinglePayload
        *vwt.add(7) = vwt_store_enum_tag as *const () as u64; // storeEnumTagSinglePayload
        *vwt.add(8) = 8;     // size = 8 bytes
        *vwt.add(9) = 8;     // stride = 8 bytes
        *vwt.add(10) = 7;    // flags: alignment=8 (alignment-1 = 7)
        *vwt.add(11) = 0;    // no extra inhabitants
    }

    // Allocate metadata with VWT pointer at [-1]
    let raw = unsafe { libc::calloc(1, 512) } as *mut u64;
    let metadata = unsafe { raw.add(2) }; // metadata[0] starts here, [-1] = VWT
    unsafe {
        *metadata.sub(1) = vwt as u64;        // VWT pointer
        *metadata = 0x200;                     // kind: struct nominal type descriptor
        *metadata.add(1) = descriptor as u64;  // nominal type descriptor

        // Read FieldOffsetVectorOffset from descriptor (+20 for struct descriptors)
        // and populate field offsets assuming pointer-sized fields.
        let desc_flags = *(descriptor as *const u32);
        let desc_kind = desc_flags & 0x1F;
        log::info!("  descriptor flags={:#x} kind={}", desc_flags, desc_kind);
        // ContextDescriptorKind: Class=16, Struct=17, Enum=18
        if desc_kind == 17 { // Struct
            // Read field descriptor relative pointer at desc+16
            let fields_rel = *((descriptor as *const i32).add(4));
            let fields_addr = (descriptor as i64 + 16 + fields_rel as i64) as *const u8;
            if fields_addr as u64 > 0x100000000 {
                let num_fields = *((fields_addr.add(12)) as *const u32);
                // FieldOffsetVectorOffset is at desc+20 for struct descriptors
                let fov_offset_raw = *((descriptor.add(20)) as *const u32);
                // Only for struct kind=17 (not class kind=16)
                if fov_offset_raw > 0 && fov_offset_raw < 100 {
                    let fov_start = fov_offset_raw as usize; // in pointer-sized words
                    // Populate field offsets: assume 8 bytes per field (pointer-sized)
                    let mut current_offset = 0u32;
                    for fi in 0..num_fields.min(16) {
                        let off_ptr = (metadata as *mut u32).add(fov_start * 2 + fi as usize);
                        *off_ptr = current_offset;
                        current_offset += 8; // each field is 8 bytes (pointer-sized)
                    }
                    // Update VWT size/stride to match total struct size
                    let total_size = (num_fields * 8) as u64;
                    *vwt.add(8) = total_size;     // size
                    *vwt.add(9) = total_size;     // stride
                    log::info!("  populated {} field offsets, total size={}", num_fields, total_size);
                }
            }
        }
    }
    let ptr = metadata as *mut u8;
    log::info!("swift_getSingletonMetadata: descriptor={:#x} -> metadata={:p}", key, ptr);
    map.insert(key, ptr);
    ptr
}

/// swift_getGenericMetadata stub: returns a valid metadata struct with VWT at [-8].
/// The real runtime's version fails because it depends on swift_conformsToProtocol.
unsafe extern "C" fn swift_get_generic_metadata_stub(
    _request: usize,
    descriptor: *const u8,
    _args: *const *const u8,
) -> *mut u8 {
    // Allocate metadata with space for VWT at [-8]
    let raw = unsafe { libc::calloc(1, 512) } as *mut u64;
    let metadata = unsafe { raw.add(8) }; // leave 64 bytes before for negative offsets

    // Create a minimal VWT at metadata[-1]
    let vwt = unsafe { libc::calloc(1, 256) } as *mut u64;
    unsafe {
        // VWT functions with correct Swift ABI signatures
        unsafe extern "C" fn vwt_initcopy(dest: *mut u8, src: *mut u8, _meta: *const u8) -> *mut u8 {
            std::ptr::copy_nonoverlapping(src, dest, 8);
            dest
        }
        unsafe extern "C" fn vwt_destroy(_: *mut u8, _: *const u8) {}
        unsafe extern "C" fn vwt_enum_tag(_: *const u8, _: u32, _: *const u8) -> u32 { 0 }
        unsafe extern "C" fn vwt_store_enum_tag(_: *mut u8, _: u32, _: u32, _: *const u8) {}

        *vwt.add(0) = vwt_initcopy as *const () as u64; // initializeBufferWithCopyOfBuffer
        *vwt.add(1) = vwt_destroy as *const () as u64; // destroy
        *vwt.add(2) = vwt_initcopy as *const () as u64; // initializeWithCopy
        *vwt.add(3) = vwt_initcopy as *const () as u64; // assignWithCopy
        *vwt.add(4) = vwt_initcopy as *const () as u64; // initializeWithTake
        *vwt.add(5) = vwt_initcopy as *const () as u64; // assignWithTake
        *vwt.add(6) = vwt_enum_tag as *const () as u64; // getEnumTagSinglePayload
        *vwt.add(7) = vwt_store_enum_tag as *const () as u64; // storeEnumTagSinglePayload
        *vwt.add(8) = 8;     // size
        *vwt.add(9) = 8;     // stride
        *vwt.add(10) = 7;    // flags (alignment-1)
    }

    unsafe {
        *metadata.sub(1) = vwt as u64;              // VWT at [-8]
        *metadata = 0x200;                            // kind: struct
        *metadata.add(1) = descriptor as u64;         // descriptor
    }

    metadata as *mut u8
}

unsafe extern "C" fn swift_isa_mask_val() -> u64 { 0x0000_7FFF_FFFF_FFF8 }

/// swift_getTypeByMangledNameInContext - look up Swift type metadata.
unsafe extern "C" fn swift_get_type_by_mangled_name(
    mangled_name: *const u8,
    mangled_name_length: usize,
    generic_env: *const u8,
    generic_args: *const *const u8,
) -> *mut u8 {
    use std::collections::HashMap;
    use std::sync::Mutex;

    // Try the REAL Swift runtime function if available, but only for names
    type RealFn = unsafe extern "C" fn(*const u8, usize, *const u8, *const *const u8) -> *mut u8;
    let real_addr = REAL_GET_TYPE_FN.load(std::sync::atomic::Ordering::SeqCst);
    if real_addr != 0 && !mangled_name.is_null() {
        let len = if mangled_name_length > 0 { mangled_name_length }
            else { let mut n = 0; while unsafe { *mangled_name.add(n) } != 0 && n < 256 { n += 1; } n };
        let has_symbolic_ref = (0..len).any(|i| {
            let b = unsafe { *mangled_name.add(i) };
            b >= 0x01 && b <= 0x1F
        });
        if has_symbolic_ref {
            log::warn!("swift_getTypeByMangledNameInContext: detected symbolic references in name (len={}), rejecting", len);
        }
        if !has_symbolic_ref {
            let real_fn: RealFn = unsafe { std::mem::transmute(real_addr) };
            let real_meta = unsafe { real_fn(mangled_name, mangled_name_length, generic_env, generic_args) };
            if !real_meta.is_null() {
                return real_meta;
            }
        } else {
            log::info!("swift_getTypeByMangledNameInContext: symbolic refs detected, proceeding to fallback path");
        }
    }

    struct SendPtr(HashMap<Vec<u8>, *mut u8>);
    unsafe impl Send for SendPtr {}
    static CACHE: Mutex<Option<SendPtr>> = Mutex::new(None);

    log::debug!("swift_getTypeByMangledNameInContext: entering fallback cache logic");

    // Extract mangled name for caching
    let key = if !mangled_name.is_null() && mangled_name_length > 0 {
        unsafe { std::slice::from_raw_parts(mangled_name, mangled_name_length as usize) }.to_vec()
    } else if !mangled_name.is_null() {
        // Null-terminated
        let mut v = Vec::new();
        let mut p = mangled_name;
        unsafe {
            while *p != 0 && v.len() < 256 { v.push(*p); p = p.add(1); }
        }
        v
    } else {
        vec![0]
    };

    // Reject symbolic references and corrupted names in fallback path too
    if key.len() > 0 {
        let has_symbolic_ref = key.iter().any(|&b| b >= 0x01 && b <= 0x1F);
        let has_mostly_control_chars = key.iter().filter(|&&b| b < 32 && b != 0).count() > key.len() / 3;

        if has_symbolic_ref {
            let name_str = String::from_utf8_lossy(&key).into_owned();
            log::info!("swift_getTypeByMangledNameInContext fallback: creating SwiftUI-compatible metadata for symbolic refs '{}' (len={})",
                      name_str, key.len());
            // Create SwiftUI-specific metadata for symbolic references
            return unsafe { create_swiftui_compatible_metadata() };
        } else if key.len() > 4 && has_mostly_control_chars {
            let name_str = String::from_utf8_lossy(&key).into_owned();
            log::warn!("swift_getTypeByMangledNameInContext fallback: corrupted name '{}' (len={})",
                      name_str, key.len());
            log::warn!("  raw bytes: {:?}", &key[..key.len().min(20)]);
            // For truly corrupted names, provide generic metadata
        }
    }

    let mut cache = CACHE.lock().unwrap();
    let map = &mut cache.get_or_insert_with(|| SendPtr(HashMap::new())).0;

    if let Some(&ptr) = map.get(&key) {
        return ptr;
    }

    // Create a minimal valid metadata struct:
    let raw = unsafe { libc::calloc(1, 512) } as *mut u64;
    let metadata = unsafe { raw.add(8) }; // Leave space for negative offsets

    // Create a minimal VWT at metadata[-1]
    let vwt = unsafe { libc::calloc(1, 256) } as *mut u64;
    unsafe {
        // VWT functions with correct Swift ABI signatures
        unsafe extern "C" fn vwt_initcopy(dest: *mut u8, src: *mut u8, _meta: *const u8) -> *mut u8 {
            std::ptr::copy_nonoverlapping(src, dest, 8);
            dest
        }
        unsafe extern "C" fn vwt_destroy(_: *mut u8, _: *const u8) {}
        unsafe extern "C" fn vwt_enum_tag(_: *const u8, _: u32, _: *const u8) -> u32 { 0 }
        unsafe extern "C" fn vwt_store_enum_tag(_: *mut u8, _: u32, _: u32, _: *const u8) {}

        *vwt.add(0) = vwt_initcopy as *const () as u64; // initializeBufferWithCopyOfBuffer
        *vwt.add(1) = vwt_destroy as *const () as u64; // destroy
        *vwt.add(2) = vwt_initcopy as *const () as u64; // initializeWithCopy
        *vwt.add(3) = vwt_initcopy as *const () as u64; // assignWithCopy
        *vwt.add(4) = vwt_initcopy as *const () as u64; // initializeWithTake
        *vwt.add(5) = vwt_initcopy as *const () as u64; // assignWithTake
        *vwt.add(6) = vwt_enum_tag as *const () as u64; // getEnumTagSinglePayload
        *vwt.add(7) = vwt_store_enum_tag as *const () as u64; // storeEnumTagSinglePayload
        *vwt.add(8) = 8;     // size
        *vwt.add(9) = 8;     // stride
        *vwt.add(10) = 7;    // flags (alignment-1)
    }

    unsafe {
        let dummy_descriptor = libc::calloc(1, 256) as *mut u64;
        // Initialize descriptor with basic required fields for SwiftUI types
        *dummy_descriptor.add(0) = 0x80000010; // flags: struct, generic
        *dummy_descriptor.add(1) = 0;          // parent (none)
        *dummy_descriptor.add(2) = 8;          // name length
        *dummy_descriptor.add(3) = 8;          // num fields
        *dummy_descriptor.add(4) = 0;          // field offset vector offset
        *metadata.sub(1) = vwt as u64;               // VWT at [-1]
        *metadata = 0x200;                            // kind: struct metadata
        *metadata.add(1) = dummy_descriptor as u64;   // descriptor pointer (non-null)
        // Fill generic arguments with self-reference so they are valid metadata pointers
        for i in 2..48 {
            *metadata.add(i) = metadata as u64;
        }
    }

    let ptr = metadata as *mut u8;
    let name_str = String::from_utf8_lossy(&key).into_owned();
    log::info!("swift_getTypeByMangledNameInContext fallback: '{}' -> {:p}", name_str, ptr);
    map.insert(key, ptr);
    ptr
}

/// Create SwiftUI-compatible metadata for symbolic references that can't be resolved
unsafe fn create_swiftui_compatible_metadata() -> *mut u8 {
    use std::sync::{Mutex, Once};

    static SWIFTUI_CACHE: Mutex<Option<usize>> = Mutex::new(None);
    static INIT_ONCE: Once = Once::new();
    static mut CACHED_METADATA: usize = 0;

    INIT_ONCE.call_once(|| {
        let metadata_ptr = unsafe { create_swiftui_metadata_internal() };
        unsafe { CACHED_METADATA = metadata_ptr as usize; }
        *SWIFTUI_CACHE.lock().unwrap() = Some(metadata_ptr as usize);
    });

    unsafe { CACHED_METADATA as *mut u8 }
}

unsafe fn create_swiftui_metadata_internal() -> *mut u8 {

    // Allocate metadata with proper SwiftUI structure
    let raw = unsafe { libc::calloc(1, 1024) } as *mut u64;
    let metadata = unsafe { raw.add(8) }; // Leave space for VWT at [-1]

    // Create SwiftUI-compatible VWT
    let vwt = unsafe { libc::calloc(1, 512) } as *mut u64;
    unsafe {
        // SwiftUI-specific VWT functions
        unsafe extern "C" fn swiftui_copy(dest: *mut u8, src: *mut u8, _meta: *const u8) -> *mut u8 {
            unsafe { std::ptr::copy_nonoverlapping(src, dest, 16) }; // SwiftUI often uses 16-byte values
            dest
        }
        unsafe extern "C" fn swiftui_destroy(_obj: *mut u8, _meta: *const u8) {}
        unsafe extern "C" fn swiftui_enum_tag(_obj: *const u8, _cases: u32, _meta: *const u8) -> u32 { 0 }

        *vwt.add(0) = swiftui_copy as *const () as u64;     // initializeBufferWithCopyOfBuffer
        *vwt.add(1) = swiftui_destroy as *const () as u64;  // destroy
        *vwt.add(2) = swiftui_copy as *const () as u64;     // initializeWithCopy
        *vwt.add(3) = swiftui_copy as *const () as u64;     // assignWithCopy
        *vwt.add(4) = swiftui_copy as *const () as u64;     // initializeWithTake
        *vwt.add(5) = swiftui_copy as *const () as u64;     // assignWithTake
        *vwt.add(6) = swiftui_enum_tag as *const () as u64; // getEnumTagSinglePayload
        *vwt.add(7) = swiftui_enum_tag as *const () as u64; // storeEnumTagSinglePayload
        *vwt.add(8) = 16;    // size (SwiftUI types often 16 bytes)
        *vwt.add(9) = 16;    // stride
        *vwt.add(10) = 15;   // flags (alignment-1 for 16-byte alignment)
    }

    // Create SwiftUI-compatible type descriptor
    let descriptor = unsafe { libc::calloc(1, 512) } as *mut u64;
    unsafe {
        *descriptor.add(0) = 0x80000050;  // flags: struct, generic, has VWT
        *descriptor.add(1) = 0;           // parent (none)
        *descriptor.add(2) = 8;           // name length
        *descriptor.add(3) = 1;           // num fields
        *descriptor.add(4) = 48;          // field offset vector offset

        // Add field offset vector at descriptor[12] (offset 48)
        *descriptor.add(12) = 0;          // field 0 at offset 0
    }

    unsafe {
        *metadata.sub(1) = vwt as u64;                    // VWT at [-1]
        *metadata.add(0) = 0x200;                         // kind: struct metadata
        *metadata.add(1) = descriptor as u64;             // type descriptor
        *metadata.add(2) = metadata as u64;               // generic argument (self-reference)
        *metadata.add(3) = safe_vwt_ptr() as u64;         // additional VWT pointer for compatibility
    }

    let result = metadata as *mut u8;
    log::info!("Created SwiftUI-compatible metadata at {:p}", result);
    result
}

unsafe extern "C" fn swift_slow_alloc(size: usize, align: usize) -> *mut u8 {
    unsafe { libc::memalign(align.max(1), size) as *mut u8 }
}
unsafe extern "C" fn swift_slow_dealloc(ptr: *mut u8, _size: usize, _align: usize) {
    if !ptr.is_null() { unsafe { libc::free(ptr as *mut _) }; }
}

/// Install safe swift_retain/release hooks via the Swift runtime's own hook mechanism.
pub fn install_swift_retain_hooks() {
    unsafe {
        // Hook retain/release via Swift's writable function pointer variables
        for (var_name, hook_fn) in [
            ("_swift_retain\0", safe_swift_retain as *const () as u64),
            ("_swift_retain_n\0", safe_swift_retain_n as *const () as u64),
            ("_swift_release\0", safe_swift_release as *const () as u64),
            ("_swift_release_n\0", safe_swift_release_n as *const () as u64),
        ] {
            let ptr = libc::dlsym(libc::RTLD_DEFAULT, var_name.as_ptr() as *const i8);
            if !ptr.is_null() {
                *(ptr as *mut u64) = hook_fn;
                log::info!("Swift hook: {} -> {:#x}", &var_name[..var_name.len()-1], hook_fn);
            }
        }

        // Re-install our SIGSEGV handler - the Swift runtime may have overridden it
        // with its own crash handler during initialization.
        unsafe extern "C" { fn grafted_reinstall_sigsegv_handler(); }
        unsafe { grafted_reinstall_sigsegv_handler(); }

        // NOTE: Binary patches for swift_checkMetadataState, swift_getAssociatedTypeWitness
        save_original_bytes();
    }
}

/// Saved original function bytes for temporary patching.
struct PatchSite {
    addr: *mut u8,
    original: [u8; 32],
    size: usize,
    replacement: *const u8,
}
unsafe impl Send for PatchSite {}
unsafe impl Sync for PatchSite {}

static PATCH_SITES: std::sync::Mutex<Vec<PatchSite>> = std::sync::Mutex::new(Vec::new());

fn save_original_bytes() {
    let legacy = std::env::var("GRAFTED_LEGACY_PATCHES").map(|v| v == "1").unwrap_or(false);

    // Permanent ABI-boundary stubs - these are necessary regardless of
    let permanent: &[(&str, usize, *const u8)] = &[
        ("swift_conformsToProtocol", 12, safe_conformsToProtocol as *const u8),
        ("swift_conformsToProtocol2", 12, safe_conformsToProtocol as *const u8),
        ("swift_conformsToProtocolCommon", 12, safe_conformsToProtocol as *const u8),
        ("swift_getWitnessTable", 12, swift_get_witness_table_stub as *const u8),
        ("swift_initStructMetadata", 12, safe_noop_return as *const u8),
        ("swift_initStructMetadataWithLayoutString", 12, safe_noop_return as *const u8),
    ];

    // Legacy defensive crashes-avoidance patches. Added empirically over
    let legacy_funcs: &[(&str, usize, *const u8)] = &[
        ("swift_checkMetadataState", 12, smart_checkMetadataState as *const u8),
        ("swift_getAssociatedTypeWitness", 12, safe_getAssociatedTypeWitness as *const u8),
        ("swift_getAssociatedConformanceWitness", 12, safe_getAssociatedConformanceWitness as *const u8),
        ("swift_getAssociatedTypeWitnessRelative", 12, safe_getAssociatedTypeWitness as *const u8),
        ("swift_getAssociatedConformanceWitnessRelative", 12, safe_getAssociatedConformanceWitness as *const u8),
        ("swift_getGenericMetadata", 12, swift_get_generic_metadata_stub as *const u8),
        ("swift_getSingletonMetadata", 12, swift_get_generic_metadata_stub as *const u8),
        ("swift_allocateGenericValueMetadata", 12, swift_get_generic_metadata_stub as *const u8),
        ("swift_allocateGenericClassMetadata", 12, swift_get_generic_metadata_stub as *const u8),
        ("swift_getKeyPath", 12, safe_return_stub_object as *const u8),
        ("swift_getOpaqueTypeMetadata", 12, swift_get_generic_metadata_stub as *const u8),
        ("swift_getOpaqueTypeMetadata2", 12, swift_get_generic_metadata_stub as *const u8),
        ("swift_getFunctionTypeMetadata", 12, swift_get_generic_metadata_stub as *const u8),
        ("swift_getTupleTypeMetadata", 12, swift_get_generic_metadata_stub as *const u8),
        ("swift_getTupleTypeMetadata2", 12, swift_get_generic_metadata_stub as *const u8),
        ("swift_getTupleTypeMetadata3", 12, swift_get_generic_metadata_stub as *const u8),
        ("swift_getExistentialTypeMetadata", 12, swift_get_generic_metadata_stub as *const u8),
        ("swift_getObjCClassMetadata", 12, swift_get_generic_metadata_stub as *const u8),
        ("swift_dynamicCast", 12, safe_return_false as *const u8),
        ("swift_dynamicCastClass", 12, safe_return_null as *const u8),
        ("swift_dynamicCastMetatype", 12, safe_return_null as *const u8),
        ("swift_dynamicCastObjCClass", 12, safe_return_null as *const u8),
        ("swift_dynamicCastUnknownClass", 12, safe_return_null as *const u8),
        ("swift_getTypeByMangledNameInContext", 12, safe_return_stub_metadata as *const u8),
        ("swift_getTypeByMangledNameInContext2", 12, safe_return_stub_metadata as *const u8),
        ("swift_getTypeByMangledNameInContextInMetadataState", 12, safe_return_stub_metadata as *const u8),
        ("swift_reportError", 12, safe_noop_return as *const u8),
        ("_swift_runtime_on_report", 12, safe_noop_return as *const u8),
        ("_swift_stdlib_reportFatalError", 12, safe_noop_return as *const u8),
        ("_swift_stdlib_reportFatalErrorInFile", 12, safe_noop_return as *const u8),
        ("swift_unexpectedError", 12, safe_noop_return as *const u8),
        ("swift_once", 12, swift_once as *const u8),
    ];

    // Merge per legacy flag. `funcs` drives the rest of this function, including
    let funcs: Vec<(&str, usize, *const u8)> = if legacy {
        permanent.iter().chain(legacy_funcs.iter()).copied().collect()
    } else {
        permanent.iter().copied().collect()
    };
    let funcs: &[(&str, usize, *const u8)] = &funcs;
    log::info!(
        "NSNotificationCenter: applying {} lifecycle patches ({}legacy mode)",
        funcs.len(),
        if legacy { "" } else { "non-" },
    );

    let mut sites = PATCH_SITES.lock().unwrap();
    for &(name, size, replacement) in funcs {
        let c_name = std::ffi::CString::new(name).unwrap();
        let addr = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c_name.as_ptr()) } as *mut u8;
        if addr.is_null() { continue; }
        let mut original = [0u8; 32];
        unsafe { std::ptr::copy_nonoverlapping(addr, original.as_mut_ptr(), size); }
        sites.push(PatchSite { addr, original, size, replacement: replacement as *const u8 });
        log::info!("Saved {} bytes from {} at {:p}", size, name, addr);
    }

    // The remaining local-offset / _nativeCopy / Slow-variant patches are
    if !legacy {
        return;
    }

    // Patch local (non-exported) C++ functions by offset from known exported symbols.
    {
        let c_name = std::ffi::CString::new("swift_getGenericMetadata").unwrap();
        let base = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c_name.as_ptr()) } as *mut u8;
        if !base.is_null() {
            let do_init = unsafe { base.add(0xEE10) };
            let mut original = [0u8; 32];
            unsafe { std::ptr::copy_nonoverlapping(do_init, original.as_mut_ptr(), 12); }
            sites.push(PatchSite { addr: do_init, original, size: 12, replacement: safe_return_stub_object as *const u8 });

            let threading_fatal = unsafe { base.add(0x3DD40) };
            let mut orig2 = [0u8; 32];
            unsafe { std::ptr::copy_nonoverlapping(threading_fatal, orig2.as_mut_ptr(), 12); }
            sites.push(PatchSite { addr: threading_fatal, original: orig2, size: 12, replacement: safe_noop_return as *const u8 });

            let await_state = unsafe { base.add(0xE9B0) };
            let mut orig3 = [0u8; 32];
            unsafe { std::ptr::copy_nonoverlapping(await_state, orig3.as_mut_ptr(), 12); }
            sites.push(PatchSite { addr: await_state, original: orig3, size: 12, replacement: safe_return_stub_object as *const u8 });
            log::info!("Saved local patches: doInit, threading::fatal, awaitState (legacy)");
        }
    }
    {
        let c_name = std::ffi::CString::new("$sSS19_copyUTF16CodeUnits4into5rangeySrys6UInt16VG_SnySiGtF").unwrap();
        let base = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c_name.as_ptr()) } as *mut u8;
        if !base.is_null() {
            let native_copy = unsafe { base.add(0x50) };
            let mut orig = [0u8; 32];
            unsafe { std::ptr::copy_nonoverlapping(native_copy, orig.as_mut_ptr(), 12); }
            sites.push(PatchSite { addr: native_copy, original: orig, size: 12, replacement: safe_noop_return as *const u8 });
        }
    }
    {
        let mut bases: Vec<usize> = Vec::new();
        if let Ok(maps) = std::fs::read_to_string("/proc/self/maps") {
            for line in maps.lines() {
                if line.contains("libswiftCore.so") && line.contains("r-xp") {
                    if let Some(addr_str) = line.split('-').next() {
                        if let Ok(addr) = usize::from_str_radix(addr_str.trim(), 16) {
                            bases.push(addr);
                        }
                    }
                }
            }
        }
        let target_off = 0x29e8d0usize;
        for &base in &bases {
            let addr = (base + target_off) as *mut u8;
            unsafe { patch_function_at(addr, safe_noop_return as *const u8) };
        }
    }

    // Slow variants (+0x30) of the associated type/conformance getters.
    if funcs.len() >= 3 {
        for &(name, _size, replacement) in &funcs[7..9] {  // 6 permanent + idx 1..3 of legacy = getAssocType/Conf
            let c_name = std::ffi::CString::new(name).unwrap();
            let addr = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c_name.as_ptr()) } as *mut u8;
            if addr.is_null() { continue; }
            let slow = unsafe { addr.add(0x30) };
            let mut original = [0u8; 32];
            unsafe { std::ptr::copy_nonoverlapping(slow, original.as_mut_ptr(), 12); }
            sites.push(PatchSite { addr: slow, original, size: 12, replacement: replacement as *const u8 });
        }
    }

    // Also build trampolines for the smart functions that need to call originals
    unsafe {
        // checkMetadataState trampoline (22-byte boundary)
        let c_name = std::ffi::CString::new("swift_checkMetadataState").unwrap();
        let orig = libc::dlsym(libc::RTLD_DEFAULT, c_name.as_ptr());
        if !orig.is_null() {
            let t = libc::mmap(std::ptr::null_mut(), 4096,
                libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS, -1, 0) as *mut u8;
            if !t.is_null() {
                std::ptr::copy_nonoverlapping(orig as *const u8, t, 22);
                let ret = (orig as usize + 22) as u64;
                *t.add(22) = 0x48; *t.add(23) = 0xB8;
                std::ptr::write_unaligned(t.add(24) as *mut u64, ret);
                *t.add(32) = 0xFF; *t.add(33) = 0xE0;
                REAL_CHECK_METADATA.store(t as u64, std::sync::atomic::Ordering::Release);
            }
        }
    }
}

/// Step 0.B: DELETED. The previous implementation did three things:
pub fn patch_binary_crash_sites() {
    // Intentionally empty. See docstring above.
}

/// Apply binary patches temporarily (before applicationDidFinishLaunching).
pub fn apply_lifecycle_patches() {
    let sites = PATCH_SITES.lock().unwrap();
    for site in sites.iter() {
        unsafe { patch_function_at(site.addr, site.replacement); }
        // Verify the patch was written
        let b0 = unsafe { std::ptr::read_volatile(site.addr) };
        let b1 = unsafe { std::ptr::read_volatile(site.addr.add(1)) };
        if b0 != 0x48 || b1 != 0xB8 {
            log::warn!("Patch at {:p} NOT applied! bytes: {:02x} {:02x}", site.addr, b0, b1);
        }
    }
    log::info!("Applied {} lifecycle patches", sites.len());
}

/// Restore original function bytes (after applicationDidFinishLaunching).
pub fn restore_lifecycle_patches() {
    let sites = PATCH_SITES.lock().unwrap();
    for site in sites.iter() {
        let page = (site.addr as usize & !0xFFF) as *mut libc::c_void;
        unsafe {
            libc::mprotect(page, 8192, libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC);
            std::ptr::copy_nonoverlapping(site.original.as_ptr(), site.addr, site.size);
            libc::mprotect(page, 8192, libc::PROT_READ | libc::PROT_EXEC);
        }
    }
    log::info!("Restored {} original functions", sites.len());
}

// Real default implementations (resolved at first call via dlsym)
static REAL_RETAIN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static REAL_RETAIN_N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static REAL_RELEASE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static REAL_RELEASE_N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn resolve_real(name: &str, cache: &std::sync::atomic::AtomicU64) -> u64 {
    let addr = cache.load(std::sync::atomic::Ordering::Relaxed);
    if addr != 0 { return addr; }
    // Look up the internal default impl (double underscore prefix)
    let real_name = format!("__{}_\0", name);
    let p = unsafe { libc::dlsym(libc::RTLD_DEFAULT, real_name.as_ptr() as *const i8) };
    let a = if p.is_null() { 1 } else { p as u64 };
    cache.store(a, std::sync::atomic::Ordering::Relaxed);
    a
}

/// Smart swift_getWitnessTable: during lifecycle calls, returns a proper witness
/// table with valid function pointers. Otherwise, pass through (for body getter).
unsafe extern "C" fn smart_getWitnessTable(a: *mut u8, _b: *mut u8, _c: *mut u8) -> *mut u8 {
    if LIFECYCLE_PATCHES_ACTIVE.load(std::sync::atomic::Ordering::Relaxed) {
        return get_stub_witness_table();
    }
    a // pass through (body getter compatibility)
}

/// Stub for swift_getWitnessTable: returns a witness table with valid metadata pointers.
unsafe extern "C" fn swift_get_witness_table_stub(_a: *mut u8, _b: *mut u8, _c: *mut u8) -> *mut u8 {
    get_stub_witness_table()
}

// All retain/release are no-ops. This prevents crashes on stub objects
unsafe extern "C" fn safe_swift_retain(object: *mut u8) -> *mut u8 { object }
unsafe extern "C" fn safe_swift_retain_n(object: *mut u8, _n: u32) -> *mut u8 { object }
unsafe extern "C" fn safe_swift_release(_object: *mut u8) {}
unsafe extern "C" fn safe_swift_release_n(_object: *mut u8, _n: u32) {}

/// Safe swift_retain/release overrides via ELF symbol interposition.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn swift_retain(object: *mut u8) -> *mut u8 {
    if object.is_null() { return object; }
    // Read refcount at +8. If it looks like our immortal marker or zero, skip.
    let rc = unsafe { *((object as *const u64).add(1)) };
    if rc == 0 || rc >= 0xFFFFFFFF00000000 { return object; }
    // Delegate to real runtime via dlsym
    type RetainFn = unsafe extern "C" fn(*mut u8) -> *mut u8;
    static REAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut addr = REAL.load(std::sync::atomic::Ordering::Relaxed);
    if addr == 0 {
        let name = std::ffi::CString::new("swift_retain").unwrap();
        let p = unsafe { libc::dlsym(libc::RTLD_NEXT, name.as_ptr()) };
        addr = if p.is_null() { 1 } else { p as u64 };
        REAL.store(addr, std::sync::atomic::Ordering::Relaxed);
    }
    if addr > 1 {
        let f: RetainFn = unsafe { std::mem::transmute(addr) };
        return unsafe { f(object) };
    }
    object
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn swift_retain_n(object: *mut u8, n: u32) -> *mut u8 {
    if object.is_null() { return object; }
    let rc = unsafe { *((object as *const u64).add(1)) };
    if rc == 0 || rc >= 0xFFFFFFFF00000000 { return object; }
    type RetainNFn = unsafe extern "C" fn(*mut u8, u32) -> *mut u8;
    static REAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut addr = REAL.load(std::sync::atomic::Ordering::Relaxed);
    if addr == 0 {
        let name = std::ffi::CString::new("swift_retain_n").unwrap();
        let p = unsafe { libc::dlsym(libc::RTLD_NEXT, name.as_ptr()) };
        addr = if p.is_null() { 1 } else { p as u64 };
        REAL.store(addr, std::sync::atomic::Ordering::Relaxed);
    }
    if addr > 1 {
        let f: RetainNFn = unsafe { std::mem::transmute(addr) };
        return unsafe { f(object, n) };
    }
    object
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn swift_release(object: *mut u8) {
    if object.is_null() { return; }
    let rc = unsafe { *((object as *const u64).add(1)) };
    if rc == 0 || rc >= 0xFFFFFFFF00000000 { return; }
    type ReleaseFn = unsafe extern "C" fn(*mut u8);
    static REAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut addr = REAL.load(std::sync::atomic::Ordering::Relaxed);
    if addr == 0 {
        let name = std::ffi::CString::new("swift_release").unwrap();
        let p = unsafe { libc::dlsym(libc::RTLD_NEXT, name.as_ptr()) };
        addr = if p.is_null() { 1 } else { p as u64 };
        REAL.store(addr, std::sync::atomic::Ordering::Relaxed);
    }
    if addr > 1 {
        let f: ReleaseFn = unsafe { std::mem::transmute(addr) };
        unsafe { f(object) };
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn swift_release_n(object: *mut u8, n: u32) {
    if object.is_null() { return; }
    let rc = unsafe { *((object as *const u64).add(1)) };
    if rc == 0 || rc >= 0xFFFFFFFF00000000 { return; }
    type ReleaseNFn = unsafe extern "C" fn(*mut u8, u32);
    static REAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut addr = REAL.load(std::sync::atomic::Ordering::Relaxed);
    if addr == 0 {
        let name = std::ffi::CString::new("swift_release_n").unwrap();
        let p = unsafe { libc::dlsym(libc::RTLD_NEXT, name.as_ptr()) };
        addr = if p.is_null() { 1 } else { p as u64 };
        REAL.store(addr, std::sync::atomic::Ordering::Relaxed);
    }
    if addr > 1 {
        let f: ReleaseNFn = unsafe { std::mem::transmute(addr) };
        unsafe { f(object, n) };
    }
}

/// Override swift_allocateWitnessTablePack via ELF symbol interposition.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn swift_allocateWitnessTablePack(
    _descriptor: *const u8,
    _pattern_args: *const *const u8,
) -> *mut *const u8 {
    // Return an array of stub pointers. The caller reads N entries from
    static STUB_PACK: std::sync::atomic::AtomicPtr<*const u8> =
        std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());
    let mut pack = STUB_PACK.load(std::sync::atomic::Ordering::Acquire);
    if pack.is_null() {
        // Allocate an array of 32 stub witness table pointers.
        let arr = unsafe { libc::calloc(32, 8) } as *mut *const u8;
        let stub_wt = get_stub_witness_table();
        for i in 0..32 {
            unsafe { *arr.add(i) = stub_wt as *const u8; }
        }
        STUB_PACK.store(arr, std::sync::atomic::Ordering::Release);
        pack = arr;
    }
    pack
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn swift_deallocateWitnessTablePack(_pack: *mut *const u8) {
    // No-op: our stub pack is statically allocated
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

// NS constant strings (dummy pointers - real apps compare by pointer identity)
static NS_CONSTANT_STRINGS: [u64; 2] = [0; 2];

// ---- Additional stubs ----

unsafe extern "C" fn block_copy(block: *mut u8) -> *mut u8 {
    if block.is_null() { return std::ptr::null_mut(); }
    // Simple: just return the block (stack blocks get "promoted" to heap - we skip that)
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
