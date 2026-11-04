//! Sanitize telemetry copies, never authoritative messages or execution inputs.
//! Typed traversal preserves non-string fields (including non-finite usage).

use super::store::redact_credential_text;
use super::types::*;

pub(super) trait Redact: Sized {
    fn redact(&mut self);
    fn redacted(mut self) -> Self {
        self.redact();
        self
    }
}

impl Redact for String {
    fn redact(&mut self) {
        *self = redact_credential_text(self);
    }
}
impl<T: Redact> Redact for Option<T> {
    fn redact(&mut self) {
        if let Some(value) = self {
            value.redact();
        }
    }
}
impl<T: Redact> Redact for Vec<T> {
    fn redact(&mut self) {
        for value in self {
            value.redact();
        }
    }
}
macro_rules! redact_fields {
    ($ty:ty: $($field:ident),+ $(,)?) => {
        impl Redact for $ty {
            fn redact(&mut self) { $(self.$field.redact();)+ }
        }
    };
}
redact_fields!(ObserveThreadIdentity: thread_key, thread_id, parent_thread_id,
    main_thread_id, thread_name);
redact_fields!(ObserveThread: identity, run_id, role, profile_color);
redact_fields!(ObserveHeader: run_id, session_id, main_thread_id, main_thread_name,
    session_name, model, thinking_level);
redact_fields!(ObserveItem: identity, run_id, event_type, text, raw, tool_call_id,
    tool_name, args, result, delegation_tool_name);
redact_fields!(ObserveLane: identity, stream_key, run_id, role, tool_name, status,
    phase, current_tool, current_path, model, event_type, summary);
redact_fields!(ObserveState: header, items, lanes, threads);
redact_fields!(ObserveRunSummary: run_id, session_id, main_thread_id, session_name,
    model, thinking_level);
impl Redact for ObserveRecorderEvent {
    fn redact(&mut self) {
        match self {
            Self::Activity { item } | Self::ActivityUpdate { item } => item.redact(),
            Self::Lane { lane } => lane.redact(),
            Self::Thread { thread } => thread.redact(),
            Self::Header { header } => header.redact(),
        }
    }
}
