//! Clipboard integration with a lossless internal fallback.
//!
//! Terminal environments without an X11/Wayland clipboard (SSH, containers,
//! headless runners) are common. Copying still works inside RustQuery via an
//! internal buffer, while `arboard` is used whenever a system clipboard exists.

#[derive(Default)]
pub struct ClipboardService {
    system: Option<arboard::Clipboard>,
    fallback: String,
}

impl ClipboardService {
    pub fn new() -> Self {
        Self::default()
    }

    fn ensure_system(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.system.is_none() {
            self.system = arboard::Clipboard::new().ok();
        }
        self.system.as_mut()
    }

    /// Store text in the internal fallback and, when possible, the system
    /// clipboard. The message is suitable for a toast/log line.
    pub fn copy(&mut self, text: &str) -> String {
        self.fallback = text.to_string();
        let bytes = text.chars().count();
        if let Some(system) = self.ensure_system() {
            if system.set_text(text.to_string()).is_ok() {
                return format!("Copied {bytes} char{}", if bytes == 1 { "" } else { "s" });
            }
        }
        format!(
            "Copied {bytes} char{} to RustQuery's internal clipboard (no system clipboard)",
            if bytes == 1 { "" } else { "s" }
        )
    }

    /// Read the system clipboard, falling back to the internal buffer.
    pub fn paste(&mut self) -> Option<String> {
        if let Some(system) = self.ensure_system() {
            if let Ok(text) = system.get_text() {
                if !text.is_empty() {
                    return Some(text);
                }
            }
        }
        if self.fallback.is_empty() {
            None
        } else {
            Some(self.fallback.clone())
        }
    }

    pub fn has_fallback(&self) -> bool {
        !self.fallback.is_empty()
    }
}

impl std::fmt::Debug for ClipboardService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipboardService")
            .field("has_system", &self.system.is_some())
            .field("fallback_len", &self.fallback.chars().count())
            .finish()
    }
}
