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
            }

            if handles.is_empty() {
                continue;
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
