//! Spacer component for adding empty space.

use std::any::Any;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::component::Component;

/// Spacer component that adds empty vertical space.
pub struct Spacer {
    height: AtomicUsize,
}

impl Spacer {
    /// Create a new spacer with the given height in rows.
    pub fn new(height: usize) -> Self {
        Self {
            height: AtomicUsize::new(height),
        }
    }

    /// Change the number of empty rows.
    pub fn set_lines(&self, height: usize) {
        self.height.store(height, Ordering::Relaxed);
    }
}

impl Default for Spacer {
    /// One empty row.
    fn default() -> Self {
        Self::new(1)
    }
}

impl Component for Spacer {
    fn render(&self, _width: usize) -> Vec<String> {
        vec![String::new(); self.height.load(Ordering::Relaxed)]
    }

    fn invalidate(&self) {
        // Spacer has no cached state
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_and_updates_empty_rows() {
        let spacer = Spacer::default();
        assert_eq!(spacer.render(10), vec![""]);
        spacer.set_lines(3);
        assert_eq!(spacer.render(10), vec!["", "", ""]);
    }
}
