//! CoreText emulation — text layout and rendering.
//!
//! Provides CTFont, CTLine, and basic glyph rendering backed by a built-in
//! bitmap font. For production use, this should be upgraded to FreeType/fontconfig.

pub mod font;
