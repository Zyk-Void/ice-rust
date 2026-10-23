//! Container component that holds multiple child components.

use std::any::Any;
use std::sync::{Arc, Mutex};

use super::component::{Component, RenderedDocument};

/// Container - a component that contains other components.
///
/// Children are rendered in order, vertically stacked.
#[derive(Clone)]
pub struct Container {
    children: Arc<Mutex<Vec<Arc<dyn Component>>>>,
}

impl Container {
    /// Create a new empty container.
    pub fn new() -> Self {
        Self {
            children: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Add a child component.
    pub fn add_child(&self, component: Arc<dyn Component>) {
        if let Ok(mut children) = self.children.lock() {
            children.push(component);
        }
    }

    /// Remove the first occurrence of a child component.
    pub fn remove_child(&self, component: &Arc<dyn Component>) {
        if let Ok(mut children) = self.children.lock() {
            if let Some(index) = children.iter().position(|c| Arc::ptr_eq(c, component)) {
                children.remove(index);
            }
        }
    }

    /// Clear all children.
    pub fn clear(&self) {
        if let Ok(mut children) = self.children.lock() {
            children.clear();
        }
    }

    /// Get the number of children.
    pub fn child_count(&self) -> usize {
        self.children.lock().map(|c| c.len()).unwrap_or(0)
    }

    /// Get a clone of the children list.
    pub fn get_children(&self) -> Vec<Arc<dyn Component>> {
        self.children.lock().map(|c| c.clone()).unwrap_or_default()
    }
}

impl Default for Container {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for Container {
    fn render(&self, width: usize) -> Vec<String> {
        // Render from a snapshot so a child may mutate this container
        // (add/remove children) without deadlocking on the lock.
        self.get_children()
            .iter()
            .flat_map(|child| child.render(width))
            .collect()
    }

    fn render_document(&self, width: usize) -> RenderedDocument {
        // Snapshot handles, then release the structural lock before rendering.
        // Streaming/input can mutate the container while a frame is prepared.
        let children = self.get_children();
        let mut document = RenderedDocument::default();
        for child in children {
            document.append(child.render_document(width));
        }
        document
    }

    fn invalidate(&self) {
        for child in self.get_children() {
            child.invalidate();
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Text;

    #[test]
    fn remove_child_drops_only_the_first_occurrence() {
        let container = Container::new();
        let child: Arc<dyn Component> = Arc::new(Text::new("x", 0, 0));
        container.add_child(child.clone());
        container.add_child(child.clone());
        container.remove_child(&child);
        assert_eq!(container.child_count(), 1);
        assert_eq!(container.render(2), vec!["x "]);
    }

    #[test]
    fn a_child_may_mutate_the_container_while_rendering() {
        struct Adder(Container);
        impl Component for Adder {
            fn render(&self, _width: usize) -> Vec<String> {
                self.0.clear();
                vec!["rendered".into()]
            }
            fn invalidate(&self) {}
            fn as_any(&self) -> &dyn Any {
                self
            }
        }
        let container = Container::new();
        container.add_child(Arc::new(Adder(container.clone())));
        assert_eq!(container.render(10), vec!["rendered"]);
        assert_eq!(container.child_count(), 0);
    }
}
