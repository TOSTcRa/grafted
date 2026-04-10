//! Core Foundation emulation.
//!
//! CF objects use reference counting and opaque pointers (CFTypeRef).
//! The type system supports toll-free bridging with ObjC (NSString ↔ CFString).

pub mod types;
pub mod string;
pub mod dictionary;
pub mod array;
pub mod data;
pub mod runloop;

pub use types::*;
