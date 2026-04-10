//! Core Graphics (Quartz 2D) emulation.
//!
//! Provides a software-rendered CGContext backed by pixel buffers.
//! For display output, buffers are blitted to X11 XImage or Wayland wl_buffer.

pub mod geometry;
pub mod color;
pub mod context;

pub use geometry::*;
