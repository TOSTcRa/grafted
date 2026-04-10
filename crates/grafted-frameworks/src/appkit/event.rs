//! NSEvent — input events routed from WindowServer to the application.

use crate::cg::geometry::*;

/// NSEventType constants
#[repr(u64)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NSEventType {
    LeftMouseDown = 1,
    LeftMouseUp = 2,
    RightMouseDown = 3,
    RightMouseUp = 4,
    MouseMoved = 5,
    LeftMouseDragged = 6,
    RightMouseDragged = 7,
    MouseEntered = 8,
    MouseExited = 9,
    KeyDown = 10,
    KeyUp = 11,
    FlagsChanged = 12,
    AppKitDefined = 13,
    SystemDefined = 14,
    ApplicationDefined = 15,
    ScrollWheel = 22,
    OtherMouseDown = 25,
    OtherMouseUp = 26,
}

/// NSEventModifierFlags
pub const NS_EVENT_MODIFIER_FLAG_CAPS_LOCK: u64 = 1 << 16;
pub const NS_EVENT_MODIFIER_FLAG_SHIFT: u64 = 1 << 17;
pub const NS_EVENT_MODIFIER_FLAG_CONTROL: u64 = 1 << 18;
pub const NS_EVENT_MODIFIER_FLAG_OPTION: u64 = 1 << 19;
pub const NS_EVENT_MODIFIER_FLAG_COMMAND: u64 = 1 << 20;

/// Internal event representation (ObjC NSEvent wraps this).
#[repr(C)]
pub struct NSEventData {
    pub event_type: NSEventType,
    pub location: CGPoint,      // in window coordinates
    pub modifier_flags: u64,
    pub timestamp: f64,
    pub window_number: i64,
    pub keycode: u16,
    pub characters: [u8; 32],   // UTF-8 key characters
    pub characters_len: u8,
}

impl NSEventData {
    pub fn new(event_type: NSEventType) -> Self {
        Self {
            event_type,
            location: CGPoint { x: 0.0, y: 0.0 },
            modifier_flags: 0,
            timestamp: 0.0,
            window_number: 0,
            keycode: 0,
            characters: [0; 32],
            characters_len: 0,
        }
    }
}
