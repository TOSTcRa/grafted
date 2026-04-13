//! Darwin framework shims — CoreFoundation, CoreGraphics, and (eventually) AppKit.
//!
//! These are C-ABI compatible implementations of macOS frameworks backed by
//! Linux-native rendering (software pixel buffers → X11/Wayland) and event
//! handling (CFRunLoop → epoll).

pub mod cf;
pub mod cg;
pub mod ct;
pub mod ws;
pub mod foundation;
pub mod appkit;
pub mod swift_runtime;
pub mod swift_sections;
pub mod swift_metadata_translate;
pub mod swift_string_abi;
pub mod registry;
