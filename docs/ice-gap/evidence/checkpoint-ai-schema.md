# AI shared-schema supplement handoff

AI-019 is complete as a source-only draft: one partial entry, 140–280 incremental Rust LOC, and twelve authored event surfaces. AI-020 is unused. AGENT-001 imports the nested provider DTOs; AI-006 is the request-option/hard-output contract and is not a substitute. RPC-004/034 are direct data consumers; RPC-003's independent closed binary schema is a related reference.

Seven complete files were independently read: TS types.ts, two focused TS test files, native types.rs/model.rs/lib.rs, and content_json.rs. The active native remote/protocol.rs181–214 read is bounded. All eight hashes match the frozen source manifest or clean frozen Git test version. Tests were read, not executed.

The supplement preserves native content/usage/event support and namespace/end_turn/cost_configured additions, then enumerates exact role-content, optional null/empty/false, serviceTiers, constrainedSampling:false and diagnostic differences. RUST-035 remains the existing stop-reason spelling defect. AI-SCHEMA-RAW-ROLE is a separately bounded source-derived public-library serialization candidate, with no reproduced production failure.

Files: spec-ai-schema-supplement.json; proposal-ai-schema-supplement.json; ai-schema-source-review.json; ai-schema-qa.json; ai-schema-defect-candidates.json; ai-schema-dependency-amendment.json. The amendment resolves DEPQA-031 and supplies the exact AI-013 source token replacement245–314 →487–514. Worker field/range/identity checks passed; coordinator review/publication is pending. No shared spec, ledger, accepted manifest, source, user-state or application/test file was changed.

Coordinator wording review completed: ThinkingLevelMap explicitly includes off through ModelThinkingLevel; Model.compat contains the complete API conditional; the model source citation is fully expanded; all three message timestamp fields explicitly use Unix milliseconds.
