# Prompt-origin host integration report

Implementation commit: `f4e6c7e3` (`fix(harness): attach trusted request context`).

Prompt origin is internal provenance for expanded user prompts. `UserMessage::with_origin` records bounded raw input, expansion kind/version, and a fingerprint of the expanded content plus timestamp. `UserMessage::original_prompt` returns the raw text only when version, size, and fingerprint validate. The host request callback validates the newest current user message, caches the result by exact `AgentMessage` for the active run, and supplies it separately as `RequestContextEvent.original_prompt`; it never substitutes expanded instructions for raw intent.

Harness skill and prompt-template invocation paths attach raw invocation text before expansion reaches the provider. The TUI direct path expands skill references before calling `prompt_message` and attaches the original editor text when expansion changes it. The actual request event is tested with an origin-bearing message (`$review`) and reports that raw intent rather than expanded instructions. Plain unexpanded prompts and legacy `<skill>`/`<prompt-template>` bodies without provenance do not claim the expanded body is original. Dedicated template, direct-TUI, queued-origin, and durable-resume integration tests remain unimplemented.

Origin remains host-only in provider-facing serialization: `ice-agent`'s `AgentMessage` adapter stores it under the session-specific `icersPromptOrigin` key, while `UserMessage`'s normal serialization skips the field. On deserialization, malformed origin metadata is discarded without rejecting an otherwise valid user message. Fingerprint mismatch and unsupported/oversized metadata are rejected as origin, not as user content. Dedicated provider-wire omission and durable-resume tests remain unimplemented.

Regression coverage:

- `ice-agent::message::prompt_origin_tests::serde_agent_message_preserves_validated_origin`
- `ice-agent::message::prompt_origin_tests::tampered_agent_message_drops_origin_and_legacy_loads`
- `ice-agent::message::prompt_origin_tests::invalid_origin_metadata_is_discarded_without_rejecting_message`
- `ice-harness::harness_extensions::request_context_waits_for_existing_tool_turn_and_uses_raw_origin`

The harness test exercises the actual provider loop with an expanded skill-like message and confirms request context receives `$review`, not the expanded instructions. Dedicated template/TUI image-heavy durable-resume tests remain follow-up coverage; no broader guarantee is claimed here.
