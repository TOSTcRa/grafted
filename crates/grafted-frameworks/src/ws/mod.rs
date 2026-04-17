//! WindowServer bridge - maps Darwin window operations to X11.

pub mod x11;
pub mod display;

pub use display::*;
