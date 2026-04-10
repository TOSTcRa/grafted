//! Darwin framework shims — CoreFoundation, CoreGraphics, and (eventually) AppKit.
//!
//! These are C-ABI compatible implementations of macOS frameworks backed by
//! Linux-native rendering (software pixel buffers → X11/Wayland) and event
//! handling (CFRunLoop → epoll).

pub mod cf;
pub mod cg;
