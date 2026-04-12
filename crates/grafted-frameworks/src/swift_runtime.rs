//! Swift runtime loader — loads the real Linux Swift runtime (libswiftCore.so)
//! and maps its symbols for Darwin binary use.
//!
//! Instead of stubbing 100+ Swift runtime functions, we load the actual Swift
//! runtime compiled for Linux. The ABI is stable since Swift 5.0, so the
//! Linux runtime works for macOS-compiled binaries.

use std::collections::HashMap;
use std::ffi::CString;
use std::sync::OnceLock;

struct SwiftRuntime {
    handles: Vec<*mut core::ffi::c_void>,
    symbols: HashMap<String, u64>,
}

unsafe impl Send for SwiftRuntime {}
unsafe impl Sync for SwiftRuntime {}

static RUNTIME: OnceLock<Option<SwiftRuntime>> = OnceLock::new();

/// Search paths for the Swift runtime libraries
const SWIFT_LIB_PATHS: &[&str] = &[
    // Extracted toolchain (our local copy)
    "swift-runtime/usr/lib/swift/linux",
    "/opt/grafted/swift/usr/lib/swift/linux",
    // System-installed Swift
    "/usr/lib/swift/linux",
    "/usr/share/swift/usr/lib/swift/linux",
    // Swiftly-installed
    "~/.local/share/swiftly/toolchains/swift-latest/usr/lib/swift/linux",
];

const SWIFT_LIBS: &[&str] = &[
    "libswiftCore.so",
    "libswiftSwiftOnoneSupport.so",
    "libswift_Concurrency.so",
    "libswiftDispatch.so",
    "libswiftGlibc.so",
    "libFoundation.so",
    "libFoundationNetworking.so",
    "libFoundationXML.so",
    "libBlocksRuntime.so",
    "libdispatch.so",
    "libswift_Backtracing.so",
    "libswift_StringProcessing.so",
    "libswift_RegexParser.so",
    "libswift_RegexBuilder.so",
];

// Our compiled SwiftUI replacement (contains App.main() that calls body)
const SWIFTUI_SHIM_PATHS: &[&str] = &[
    "shims/libSwiftUI.so",
    "/opt/grafted/shims/libSwiftUI.so",
];

/// Try to load the real Swift runtime. Returns symbol count or 0 on failure.
pub fn load_swift_runtime() -> usize {
    let rt = RUNTIME.get_or_init(|| {
        for base_path in SWIFT_LIB_PATHS {
            let expanded = if base_path.starts_with('~') {
                if let Some(home) = std::env::var("HOME").ok() {
                    base_path.replacen('~', &home, 1)
                } else {
                    continue;
                }
            } else {
                base_path.to_string()
            };

            let core_path = format!("{}/libswiftCore.so", expanded);
            if !std::path::Path::new(&core_path).exists() {
                continue;
            }

            log::info!("Swift runtime: found at {}", expanded);
            let mut handles = Vec::new();
            let mut symbols = HashMap::new();

            for lib_name in SWIFT_LIBS {
                let lib_path = format!("{}/{}", expanded, lib_name);
                let c_path = CString::new(lib_path.as_str()).unwrap();
                let handle = unsafe { libc::dlopen(c_path.as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL) };
                if handle.is_null() {
                    let err = unsafe { std::ffi::CStr::from_ptr(libc::dlerror()) };
                    log::debug!("Swift runtime: {} — {}", lib_name, err.to_string_lossy());
                    continue;
                }
                handles.push(handle);
                log::info!("Swift runtime: loaded {}", lib_name);

                // No binary patches — the metadata translation layer
                // (swift_metadata_translate.rs) fixes the data at load time.
            } // end for lib_name in SWIFT_LIBS

            if handles.is_empty() {
                continue;
            }

            // Load our compiled SwiftUI replacement (App.main() that calls body)
            for shim_path in SWIFTUI_SHIM_PATHS {
                if !std::path::Path::new(shim_path).exists() { continue; }
                let c_path = CString::new(*shim_path).unwrap();
                let handle = unsafe { libc::dlopen(c_path.as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL) };
                if handle.is_null() {
                    let err = unsafe { std::ffi::CStr::from_ptr(libc::dlerror()) };
                    log::warn!("SwiftUI shim: {} — {}", shim_path, err.to_string_lossy());
                    continue;
                }
                handles.push(handle);
                log::info!("SwiftUI shim: loaded {}", shim_path);

                // Extract ALL symbols from the SwiftUI shim (including mangled Swift names)
                // Use dlsym for known critical symbols
                for sym_name in &[
                    "$s7SwiftUI3AppPAAE4mainyyFZ",
                    "$s7SwiftUI12SceneBuilderV10buildBlockyxxAA0C0RzlFZ",
                    "$s7SwiftUI11WindowGroupVMa",
                    "$s7SwiftUI12MenuBarExtraVMa",
                ] {
                    let c_name = CString::new(*sym_name).unwrap();
                    let addr = unsafe { libc::dlsym(handle, c_name.as_ptr()) };
                    if !addr.is_null() {
                        symbols.insert(format!("_{}", sym_name), addr as u64);
                        symbols.insert(sym_name.to_string(), addr as u64);
                    }
                }

                // Bulk-extract all exported symbols by scanning the dynamic symbol table
                // For the SwiftUI shim, just grab everything with $s7SwiftUI prefix
                for &known in SWIFTUI_SYMBOLS {
                    let c_name = CString::new(known).unwrap();
                    let addr = unsafe { libc::dlsym(handle, c_name.as_ptr()) };
                    if !addr.is_null() {
                        symbols.insert(format!("_{}", known), addr as u64);
                        symbols.insert(known.to_string(), addr as u64);
                    }
                }

                break;
            }

            // Extract all swift_* symbols from the loaded libraries
            for &sym_name in SWIFT_SYMBOLS {
                let c_name = CString::new(sym_name).unwrap();
                for &handle in &handles {
                    let addr = unsafe { libc::dlsym(handle, c_name.as_ptr()) };
                    if !addr.is_null() {
                        // Darwin symbols have leading underscore
                        symbols.insert(format!("_{}", sym_name), addr as u64);
                        symbols.insert(sym_name.to_string(), addr as u64);
                        break;
                    }
                }
            }

            log::info!("Swift runtime: resolved {} symbols", symbols.len());
            return Some(SwiftRuntime { handles, symbols });
        }

        log::warn!("Swift runtime: not found — using stubs (SwiftUI apps won't render)");
        None
    });

    rt.as_ref().map(|r| r.symbols.len()).unwrap_or(0)
}

/// Get the loaded Swift runtime symbol table for framework registry merging.
pub fn swift_symbols() -> HashMap<String, u64> {
    load_swift_runtime();
    RUNTIME.get()
        .and_then(|r| r.as_ref())
        .map(|r| r.symbols.clone())
        .unwrap_or_default()
}

/// Critical Swift runtime symbols to resolve
const SWIFT_SYMBOLS: &[&str] = &[
    // Object lifecycle
    "swift_retain", "swift_release", "swift_retain_n", "swift_release_n",
    "swift_allocObject", "swift_deallocObject", "swift_initStackObject",
    "swift_setDeallocating", "swift_isUniquelyReferenced_nonNull_native",
    "swift_isUniquelyReferenced_native",
    // Bridge objects
    "swift_bridgeObjectRetain", "swift_bridgeObjectRelease",
    "swift_bridgeObjectRetain_n", "swift_bridgeObjectRelease_n",
    // Unowned/weak refs
    "swift_unownedRetain", "swift_unownedRelease", "swift_unownedRetainStrong",
    "swift_weakInit", "swift_weakDestroy", "swift_weakAssign",
    "swift_weakLoadStrong", "swift_weakTakeInit", "swift_weakTakeAssign",
    "swift_weakCopyInit", "swift_weakCopyAssign",
    "swift_unknownObjectWeakInit", "swift_unknownObjectWeakAssign",
    "swift_unknownObjectWeakDestroy", "swift_unknownObjectWeakLoadStrong",
    // Metadata
    "swift_getTypeByMangledNameInContext", "swift_getTypeByMangledNameInContext2",
    "swift_getTypeByMangledNameInContextInMetadataState",
    "swift_getTypeByMangledNameInContextInMetadataState2",
    "swift_getSingletonMetadata", "swift_getGenericMetadata",
    "swift_getForeignTypeMetadata", "swift_getObjCClassMetadata",
    "swift_getExistentialTypeMetadata",
    "swift_checkMetadataState", "swift_initClassMetadata2",
    "swift_initStructMetadata", "swift_updateClassMetadata2",
    "swift_allocateGenericClassMetadata", "swift_allocateGenericValueMetadata",
    // Witness tables
    "swift_getWitnessTable", "swift_getAssociatedTypeWitness",
    "swift_getAssociatedConformanceWitness",
    "swift_conformsToProtocol",
    // Function/tuple metadata
    "swift_getFunctionTypeMetadata", "swift_getFunctionTypeMetadata0",
    "swift_getFunctionTypeMetadata2",
    "swift_getTupleTypeMetadata", "swift_getTupleTypeMetadata2",
    "swift_getTupleTypeMetadata3",
    "swift_getMetatypeMetadata",
    // Opaque types
    "swift_getOpaqueTypeMetadata", "swift_getOpaqueTypeMetadata2",
    "swift_getOpaqueTypeConformance", "swift_getOpaqueTypeConformance2",
    // Dynamic cast
    "swift_dynamicCast", "swift_dynamicCastClass",
    "swift_dynamicCastMetatype", "swift_dynamicCastObjCClass",
    "swift_dynamicCastObjCProtocolConditional", "swift_dynamicCastUnknownClass",
    // Memory
    "swift_slowAlloc", "swift_slowDealloc",
    "swift_allocBox", "swift_deallocBox", "swift_projectBox", "swift_makeBoxUnique",
    // Error handling
    "swift_allocError", "swift_errorRetain", "swift_errorRelease",
    "swift_getErrorValue", "swift_willThrow", "swift_unexpectedError",
    // Enum
    "swift_getEnumCaseMultiPayload", "swift_getEnumTagSinglePayloadGeneric",
    "swift_storeEnumTagMultiPayload", "swift_storeEnumTagSinglePayloadGeneric",
    // Array
    "swift_arrayDestroy", "swift_arrayInitWithCopy",
    "swift_arrayInitWithTakeBackToFront", "swift_arrayInitWithTakeFrontToBack",
    // Key paths
    "swift_getKeyPath", "swift_getAtKeyPath",
    // Once
    "swift_once",
    // Access
    "swift_beginAccess", "swift_endAccess",
    // ObjC interop
    "swift_getInitializedObjCClass", "swift_getObjCClassFromMetadata",
    "swift_unknownObjectRetain", "swift_unknownObjectRelease",
    // Concurrency
    "swift_task_create", "swift_task_alloc", "swift_task_dealloc",
    "swift_task_switch", "swift_task_enqueue",
    "swift_task_getCurrent", "swift_task_getMainExecutor",
    "swift_task_isCurrentExecutor",
    "swift_job_run",
    "swift_continuation_init", "swift_continuation_resume",
    "swift_continuation_throwingResume", "swift_continuation_throwingResumeWithError",
    "swift_asyncLet_begin", "swift_asyncLet_end",
    "swift_taskGroup_initialize", "swift_taskGroup_destroy",
    // Misc
    "swift_deletedMethodError",
    "swift_lookUpClassMethod",
    "swift_isEscapingClosureAtFileLocation",
    "swift_stdlib_random",
    "swift_coroFrameAlloc",
];

/// Symbols exported by our compiled SwiftUI shim (shims/libSwiftUI.so)
const SWIFTUI_SYMBOLS: &[&str] = &[
    "$s7SwiftUI3AppPAAE4mainyyFZ",
    "$s7SwiftUI3AppMp",
    "$s7SwiftUI3AppP4BodyAC_AA5SceneTn",
    "$s7SwiftUI3AppTL",
    "$s7SwiftUI5SceneMp",
    "$s7SwiftUI4ViewMp",
    "$s7SwiftUI12SceneBuilderV10buildBlockyxxAA0C0RzlFZ",
    "$s7SwiftUI12SceneBuilderV10buildBlockyAA05TupleC0Vyxq_Gx_q_tAA0C0RzAaHR_r0_lFZ",
    "$s7SwiftUI12SceneBuilderVMa",
    "$s7SwiftUI12SceneBuilderVMn",
    "$s7SwiftUI12SceneBuilderVN",
    "$s7SwiftUI11ViewBuilderV10buildBlockyxxAA0C0RzlFZ",
    "$s7SwiftUI11ViewBuilderV10buildBlockAA05EmptyC0VyFZ",
    "$s7SwiftUI11ViewBuilderVMa",
    "$s7SwiftUI11ViewBuilderVMn",
    "$s7SwiftUI11ViewBuilderVN",
    "$s7SwiftUI11WindowGroupV7contentACyxGxyXE_tcfC",
    "$s7SwiftUI11WindowGroupV_7contentACyxGSS_xyXEtcfC",
    "$s7SwiftUI11WindowGroupVMa",
    "$s7SwiftUI11WindowGroupVMn",
    "$s7SwiftUI11WindowGroupVyxGAA5SceneAAMc",
    "$s7SwiftUI11WindowGroupVyxGAA5SceneAAWP",
    "$s7SwiftUI12MenuBarExtraV_11systemImage7contentACyxq_GSS_SSq_yXEtcfC",
    "$s7SwiftUI12MenuBarExtraV_11systemImage10isInserted7contentACyxq_GSS_SSAA7BindingVySbGq_yXEtcfC",
    "$s7SwiftUI12MenuBarExtraV7content5labelACyxq_Gq_yXE_xyXEtcfC",
    "$s7SwiftUI12MenuBarExtraVMa",
    "$s7SwiftUI12MenuBarExtraVMn",
    "$s7SwiftUI12MenuBarExtraVyxq_GAA5SceneAAMc",
    "$s7SwiftUI12MenuBarExtraVyxq_GAA5SceneAAWP",
    "$s7SwiftUI10ScenePhaseOMa",
    "$s7SwiftUI10ScenePhaseOMn",
    "$s7SwiftUI10ScenePhaseON",
    "$s7SwiftUI10ScenePhaseOSHAAMc",
    "$s7SwiftUI10ScenePhaseOSQAAMc",
    "$s7SwiftUI10TupleSceneVMa",
    "$s7SwiftUI10TupleSceneVyxq_GAA0D0AAMc",
    "$s7SwiftUI10TupleSceneVyxq_GAA0D0AAWP",
    "$s7SwiftUI4TextVyACSScfC",
    "$s7SwiftUI4TextVMa",
    "$s7SwiftUI4TextVAA4ViewAAMc",
    "$s7SwiftUI4TextVAA4ViewAAWP",
    "$s7SwiftUI9EmptyViewVACycfC",
    "$s7SwiftUI9EmptyViewVMa",
    "$s7SwiftUI9EmptyViewVAA0D0AAMc",
    "$s7SwiftUI5StateVMa",
    "$s7SwiftUI5StateV12wrappedValueACyxGx_tcfC",
    "$s7SwiftUI5StateV12wrappedValuexvg",
    "$s7SwiftUI7BindingVMa",
    "$s7SwiftUI7BindingV3get3setACyxGxyc_yxctcfC",
    "$s7SwiftUI11EnvironmentVMa",
    "$s7SwiftUI17EnvironmentValuesVMa",
    "$s7SwiftUI17EnvironmentValuesV10scenePhaseAA05SceneF0Ovg",
    "$s7SwiftUI28NSApplicationDelegateAdaptorVMa",
    "$s7SwiftUI28NSApplicationDelegateAdaptorVMn",
    "$s7SwiftUI28NSApplicationDelegateAdaptorVyACyxGxmcfC",
    "$s7SwiftUI15ModifiedContentVMa",
    "$s7SwiftUI15ModifiedContentVyxq_GAA4ViewAAMc",
    "$s7SwiftUI12ViewModifierMp",
    "$s7SwiftUI12ViewModifierP4BodyAC_AA0C0Tn",
    "$s7SwiftUI21_ViewModifier_ContentVMa",
];
