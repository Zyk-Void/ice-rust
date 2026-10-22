//! Clipboard-owner lifetime tests: no OS clipboard, terminal, or environment access.

use super::*;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Default)]
struct BackendState {
    constructed: usize,
    dropped: usize,
    writes: Vec<String>,
}

struct FakeClipboard(Rc<RefCell<BackendState>>);

impl FakeClipboard {
    fn new(state: &Rc<RefCell<BackendState>>) -> Self {
        state.borrow_mut().constructed += 1;
        Self(state.clone())
    }

    fn write(&mut self, text: &str) -> Result<(), ()> {
        self.0.borrow_mut().writes.push(text.into());
        Ok(())
    }
}

impl Drop for FakeClipboard {
    fn drop(&mut self) {
        self.0.borrow_mut().dropped += 1;
    }
}

#[test]
fn clipboard_owner_survives_copy_reuses_backend_and_drops_only_at_slot_teardown() {
    // Rc intentionally models a backend with no Send guarantee.
    let state = Rc::new(RefCell::new(BackendState::default()));
    let slot = RefCell::new(None);
    assert!(write_retained_resource(
        &slot,
        || Ok(FakeClipboard::new(&state)),
        |backend| backend.write("first selection"),
    ));
    assert!(slot.borrow().is_some());
    assert_eq!(state.borrow().constructed, 1);
    assert_eq!(state.borrow().dropped, 0);
    assert!(write_retained_resource(
        &slot,
        || panic!("a second copy must reuse the retained owner"),
        |backend| backend.write("second selection"),
    ));
    assert_eq!(state.borrow().constructed, 1);
    assert_eq!(state.borrow().dropped, 0);
    assert_eq!(
        state.borrow().writes,
        ["first selection", "second selection"]
    );
    drop(slot);
    assert_eq!(state.borrow().dropped, 1);
}

#[test]
fn clipboard_initialization_failure_returns_false_and_allows_retry() {
    let state = Rc::new(RefCell::new(BackendState::default()));
    let slot = RefCell::new(None);
    assert!(!write_retained_resource(
        &slot,
        || Err::<FakeClipboard, ()>(()),
        |_| panic!("failed initialization must not attempt a write"),
    ));
    assert!(slot.borrow().is_none());
    assert_eq!(state.borrow().constructed, 0);
    assert!(write_retained_resource(
        &slot,
        || Ok(FakeClipboard::new(&state)),
        |backend| backend.write("retried selection"),
    ));
    assert_eq!(state.borrow().constructed, 1);
    assert_eq!(state.borrow().dropped, 0);
    assert_eq!(state.borrow().writes, ["retried selection"]);
}

#[test]
fn clipboard_failed_write_discards_owner_and_next_copy_reinitializes() {
    let state = Rc::new(RefCell::new(BackendState::default()));
    let slot = RefCell::new(None);
    assert!(write_retained_resource(
        &slot,
        || Ok(FakeClipboard::new(&state)),
        |backend| backend.write("first selection"),
    ));
    assert!(!write_retained_resource(
        &slot,
        || panic!("reuse the initialized backend before observing its failure"),
        |_| Err(()),
    ));
    assert!(slot.borrow().is_none());
    assert_eq!(state.borrow().constructed, 1);
    assert_eq!(state.borrow().dropped, 1);
    assert!(write_retained_resource(
        &slot,
        || Ok(FakeClipboard::new(&state)),
        |backend| backend.write("replacement owner selection"),
    ));
    assert_eq!(state.borrow().constructed, 2);
    assert_eq!(state.borrow().dropped, 1);
    assert_eq!(
        state.borrow().writes,
        ["first selection", "replacement owner selection"]
    );
    drop(slot);
    assert_eq!(state.borrow().dropped, 2);
}

#[test]
fn linux_copy_prefers_platform_tools_over_native_clipboard() {
    // Regression: on Hyprland arboard's X11 write "succeeded" without reaching
    // Wayland clients, so `wl-copy` never ran and nothing was copied.
    let calls = RefCell::new(Vec::new());
    let native = |_: &str| {
        calls.borrow_mut().push("native");
        true
    };
    let commands = |_: &str| {
        calls.borrow_mut().push("commands");
        true
    };
    assert!(copy_locally("x", true, native, commands));
    assert_eq!(*calls.borrow(), ["commands"]);

    calls.borrow_mut().clear();
    assert!(copy_locally("x", false, native, commands));
    assert_eq!(*calls.borrow(), ["native"]);

    // A missing tool still falls back to the native clipboard.
    calls.borrow_mut().clear();
    let no_tools = |_: &str| {
        calls.borrow_mut().push("commands");
        false
    };
    assert!(copy_locally("x", true, native, no_tools));
    assert_eq!(*calls.borrow(), ["commands", "native"]);
}
