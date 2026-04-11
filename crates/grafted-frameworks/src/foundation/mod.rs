//! Foundation framework — NSString, NSArray, NSDictionary, NSBundle, NSNotificationCenter.
//!
//! Most Foundation types are toll-free bridged to Core Foundation:
//!   NSString ↔ CFString, NSDictionary ↔ CFDictionary, NSArray ↔ CFArray
//!
//! We implement them as ObjC classes that wrap our CF types, so
//! objc_msgSend dispatches to methods that operate on CF internals.

pub mod string;
pub mod bundle;
pub mod notification;
pub mod register;

pub fn register_classes() {
    register::register_all();
}
