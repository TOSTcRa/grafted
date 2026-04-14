//! Maccy-like clipboard manager UI rendered natively via CGContext.
//!
//! TEMPORARY: This bypasses applicationDidFinishLaunching (which crashes on
//! unresolved Swift witness table packs) and renders the UI directly in Rust.
//! Will be deprecated when applicationDidFinishLaunching works natively.

use crate::cg::context::*;
use crate::cg::geometry::*;
use crate::ct::font::draw_text_bitmap;

/// Colors matching Maccy's dark theme
const BG_COLOR: (f64, f64, f64) = (0.118, 0.118, 0.118);       // #1E1E1E
const TITLE_BAR: (f64, f64, f64) = (0.157, 0.157, 0.157);      // #282828
const ROW_EVEN: (f64, f64, f64) = (0.149, 0.149, 0.149);       // #262626
const ROW_ODD: (f64, f64, f64) = (0.133, 0.133, 0.133);        // #222222
const ROW_HOVER: (f64, f64, f64) = (0.20, 0.20, 0.25);         // bluish highlight
const SEARCH_BG: (f64, f64, f64) = (0.098, 0.098, 0.098);      // #191919
const SEARCH_BORDER: (f64, f64, f64) = (0.25, 0.25, 0.25);     // #404040
const TEXT_PRIMARY: [f64; 4] = [0.88, 0.88, 0.88, 1.0];        // #E0E0E0
const TEXT_SECONDARY: [f64; 4] = [0.50, 0.50, 0.50, 1.0];      // #808080
const TEXT_ACCENT: [f64; 4] = [0.35, 0.60, 1.0, 1.0];          // #5999FF
const DIVIDER: (f64, f64, f64) = (0.22, 0.22, 0.22);           // #383838
const FOOTER_BG: (f64, f64, f64) = (0.11, 0.11, 0.11);         // #1C1C1C

const TITLE_BAR_H: f64 = 28.0;
const SEARCH_BAR_H: f64 = 36.0;
const ROW_H: f64 = 32.0;
const FOOTER_H: f64 = 28.0;

use super::clipboard_history;

/// Render the full Maccy UI into a CGContext.
pub fn render(ctx: CGContextRef, entries: &[clipboard_history::ClipboardEntry], selected: usize, search_query: &str) {
    if ctx.is_null() { return; }

    let w = unsafe { CGBitmapContextGetWidth(ctx) } as f64;
    let h = unsafe { CGBitmapContextGetHeight(ctx) } as f64;

    // Background
    fill_rect(ctx, 0.0, 0.0, w, h, BG_COLOR);

    // Title bar
    fill_rect(ctx, 0.0, 0.0, w, TITLE_BAR_H, TITLE_BAR);
    // Traffic lights
    let dots = [(1.0,0.38,0.34), (1.0,0.74,0.17), (0.21,0.78,0.35)];
    for (i, c) in dots.iter().enumerate() {
        fill_rect(ctx, 8.0 + i as f64 * 20.0, 8.0, 12.0, 12.0, *c);
    }
    draw_text_bitmap(ctx, "Maccy", w / 2.0 - 16.0, 8.0, [0.75, 0.75, 0.75, 1.0], 1.0);

    // Search bar
    let sy = TITLE_BAR_H;
    fill_rect(ctx, 0.0, sy, w, SEARCH_BAR_H, SEARCH_BG);
    // Search field
    fill_rect(ctx, 8.0, sy + 6.0, w - 16.0, 24.0, (0.16, 0.16, 0.16));
    // Border
    fill_rect(ctx, 8.0, sy + 6.0, w - 16.0, 1.0, SEARCH_BORDER);
    fill_rect(ctx, 8.0, sy + 29.0, w - 16.0, 1.0, SEARCH_BORDER);
    fill_rect(ctx, 8.0, sy + 6.0, 1.0, 24.0, SEARCH_BORDER);
    fill_rect(ctx, w - 9.0, sy + 6.0, 1.0, 24.0, SEARCH_BORDER);
    // Search icon (magnifying glass text)
    draw_text_bitmap(ctx, "Q", 16.0, sy + 11.0, TEXT_SECONDARY, 1.0);
    // Search text or placeholder
    if search_query.is_empty() {
        draw_text_bitmap(ctx, "Search...", 30.0, sy + 11.0, TEXT_SECONDARY, 1.0);
    } else {
        draw_text_bitmap(ctx, search_query, 30.0, sy + 11.0, TEXT_PRIMARY, 1.0);
    }

    // Divider
    fill_rect(ctx, 0.0, sy + SEARCH_BAR_H, w, 1.0, DIVIDER);

    // Clipboard list area
    let list_y = TITLE_BAR_H + SEARCH_BAR_H + 1.0;
    let list_h = h - list_y - FOOTER_H;
    let max_visible = (list_h / ROW_H) as usize;

    if entries.is_empty() {
        // Empty state
        draw_text_bitmap(ctx, "No clipboard history", w / 2.0 - 60.0, list_y + list_h / 2.0 - 8.0, TEXT_SECONDARY, 1.0);
        draw_text_bitmap(ctx, "Copy something to get started", w / 2.0 - 88.0, list_y + list_h / 2.0 + 10.0, TEXT_SECONDARY, 1.0);
    } else {
        for (i, entry) in entries.iter().enumerate().take(max_visible) {
            let ry = list_y + i as f64 * ROW_H;
            if ry + ROW_H > h - FOOTER_H { break; }

            // Row background
            let bg = if i == selected {
                ROW_HOVER
            } else if i % 2 == 0 {
                ROW_EVEN
            } else {
                ROW_ODD
            };
            fill_rect(ctx, 0.0, ry, w, ROW_H, bg);

            // Pin indicator
            let text_x = if entry.pinned {
                draw_text_bitmap(ctx, "*", 8.0, ry + 10.0, TEXT_ACCENT, 1.0);
                22.0
            } else {
                12.0
            };

            // Truncated text (first ~50 chars)
            let display_text: String = entry.text.chars()
                .filter(|c| !c.is_control())
                .take(55)
                .collect();
            let display_text = if entry.text.len() > 55 {
                format!("{}...", display_text)
            } else {
                display_text
            };
            draw_text_bitmap(ctx, &display_text, text_x, ry + 10.0, TEXT_PRIMARY, 1.0);

            // Time ago (right-aligned)
            let time_str = entry.time_ago();
            let time_w = time_str.len() as f64 * 6.0;
            draw_text_bitmap(ctx, &time_str, w - time_w - 10.0, ry + 10.0, TEXT_SECONDARY, 0.9);

            // Divider between rows
            fill_rect(ctx, 0.0, ry + ROW_H - 1.0, w, 1.0, DIVIDER);
        }
    }

    // Footer
    let fy = h - FOOTER_H;
    fill_rect(ctx, 0.0, fy, w, FOOTER_H, FOOTER_BG);
    fill_rect(ctx, 0.0, fy, w, 1.0, DIVIDER);

    // Footer text
    let count_text = if entries.is_empty() {
        "Empty".to_string()
    } else {
        format!("{} items", entries.len())
    };
    draw_text_bitmap(ctx, &count_text, 10.0, fy + 8.0, TEXT_SECONDARY, 0.9);

    // Keyboard hints
    draw_text_bitmap(ctx, "Esc: quit", w - 60.0, fy + 8.0, TEXT_SECONDARY, 0.9);
}

/// Create initial history by reading the current system clipboard
pub fn initial_history() -> clipboard_history::ClipboardHistory {
    let mut history = clipboard_history::ClipboardHistory::new();
    history.poll(); // Read current clipboard content
    history
}

fn fill_rect(ctx: CGContextRef, x: f64, y: f64, w: f64, h: f64, color: (f64, f64, f64)) {
    unsafe {
        CGContextSetRGBFillColor(ctx, color.0, color.1, color.2, 1.0);
        CGContextFillRect(ctx, CGRect {
            origin: CGPoint { x, y },
            size: CGSize { width: w, height: h },
        });
    }
}
