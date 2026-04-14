//! Clipboard history — polls the system clipboard and maintains history.
//!
//! Uses xclip/xsel to read the system clipboard (reliable, no X11 protocol complexity).
//! Background thread polls every 500ms and updates the history.

use std::sync::{Arc, Mutex};
use std::time::Instant;

/// A single clipboard entry
#[derive(Clone)]
pub struct ClipboardEntry {
    pub text: String,
    pub timestamp: Instant,
    pub pinned: bool,
}

impl ClipboardEntry {
    pub fn time_ago(&self) -> String {
        let secs = self.timestamp.elapsed().as_secs();
        if secs < 5 { return "now".into(); }
        if secs < 60 { return format!("{}s", secs); }
        if secs < 3600 { return format!("{}m", secs / 60); }
        if secs < 86400 { return format!("{}h", secs / 3600); }
        format!("{}d", secs / 86400)
    }
}

/// Shared clipboard history state
pub struct ClipboardHistory {
    pub entries: Vec<ClipboardEntry>,
    last_content: String,
    max_entries: usize,
}

impl ClipboardHistory {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            last_content: String::new(),
            max_entries: 200,
        }
    }

    /// Check the system clipboard and add new content if changed
    pub fn poll(&mut self) {
        if let Some(content) = read_system_clipboard() {
            let trimmed = content.trim().to_string();
            if !trimmed.is_empty() && trimmed != self.last_content {
                self.last_content = trimmed.clone();
                // Don't add duplicates
                if !self.entries.iter().any(|e| e.text == trimmed) {
                    self.entries.insert(0, ClipboardEntry {
                        text: trimmed,
                        timestamp: Instant::now(),
                        pinned: false,
                    });
                    // Evict oldest (non-pinned) if over limit
                    while self.entries.len() > self.max_entries {
                        if let Some(pos) = self.entries.iter().rposition(|e| !e.pinned) {
                            self.entries.remove(pos);
                        } else {
                            break;
                        }
                    }
                }
            }
        }
    }
}

/// Read the system clipboard using xclip or xsel
fn read_system_clipboard() -> Option<String> {
    // Try xclip first
    if let Ok(output) = std::process::Command::new("xclip")
        .args(&["-selection", "clipboard", "-o"])
        .output()
    {
        if output.status.success() {
            return Some(String::from_utf8_lossy(&output.stdout).into_owned());
        }
    }
    // Fall back to xsel
    if let Ok(output) = std::process::Command::new("xsel")
        .args(&["--clipboard", "--output"])
        .output()
    {
        if output.status.success() {
            return Some(String::from_utf8_lossy(&output.stdout).into_owned());
        }
    }
    None
}

/// Write to the system clipboard
pub fn write_system_clipboard(text: &str) -> bool {
    // Try xclip
    use std::io::Write;
    if let Ok(mut child) = std::process::Command::new("xclip")
        .args(&["-selection", "clipboard", "-i"])
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
        return true;
    }
    // Fall back to xsel
    if let Ok(mut child) = std::process::Command::new("xsel")
        .args(&["--clipboard", "--input"])
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
        return true;
    }
    false
}

/// Start clipboard polling in a background thread
pub fn start_polling(history: Arc<Mutex<ClipboardHistory>>) {
    std::thread::spawn(move || {
        loop {
            {
                if let Ok(mut h) = history.lock() {
                    h.poll();
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    });
}
