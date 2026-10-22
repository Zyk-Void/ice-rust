# Current ice-rust Gap Inventory

Exhaustive index of the contract entries in [`README.md`](README.md). The table includes every audited contract: incomplete items are gaps; `different-by-design` items are listed separately and are not counted as gaps. Each contract ID links to its full area-specific specification, evidence, and comparison.

**791 contracts:** 351 missing, 404 partial, 36 different by design. **Remaining gaps: 755.**

| Area | Contracts | Missing | Partial | Different by design |
|---|---:|---:|---:|---:|
| AI | 50 | 23 | 26 | 1 |
| AGENT | 65 | 5 | 49 | 11 |
| TOOLS | 31 | 10 | 21 | 0 |
| SESSION | 38 | 4 | 27 | 7 |
| SET | 33 | 1 | 31 | 1 |
| EXT | 57 | 8 | 46 | 3 |
| RPC | 36 | 25 | 10 | 1 |
| TUI | 151 | 37 | 114 | 0 |
| SUB | 44 | 33 | 11 | 0 |
| SAFE | 26 | 26 | 0 | 0 |
| OBS | 29 | 28 | 1 | 0 |
| COG | 68 | 68 | 0 | 0 |
| BLACK | 41 | 33 | 8 | 0 |
| AUTH | 33 | 14 | 19 | 0 |
| CLI | 46 | 16 | 24 | 6 |
| MISC | 43 | 20 | 17 | 6 |

## AI

| ID | Contract | Status | Priority | Estimated Rust lines |
|---|---|---|---|---:|
| [AI-001](ai.md#ai-001---shared-assistant-retry-classification-bounded-delay-and-lifecycle-callbacks) | Shared assistant retry classification, bounded delay and lifecycle callbacks | partial | P1 | 160–300 |
| [AI-002](ai.md#ai-002---provider-collection-with-cache-first-refresh-and-generation-checked-publication) | Provider collection with cache-first refresh and generation-checked publication | partial | P1 | 500–950 |
| [AI-003](ai.md#ai-003---api-factory-dispatch-lazy-setup-errors-and-legacy-registry-compatibility) | API factory dispatch, lazy setup errors and legacy registry compatibility | partial | P1 | 230–480 |
| [AI-004](ai.md#ai-004---provider-request-retry-headers-delay-validation-and-typed-error-preservation) | Provider request retry headers, delay validation and typed error preservation | partial | P1 | 150–320 |
| [AI-005](ai.md#ai-005---thinking-level-availability-ultra-and-upward-first-clamping) | Thinking-level availability, ultra and upward-first clamping | partial | P1 | 90–210 |
| [AI-006](ai.md#ai-006---shared-request-options-and-non-widening-hard-output-authority) | Shared request options and non-widening hard output authority | partial | P0 | 350–700 |
| [AI-007](ai.md#ai-007---provider-overflow-and-prematurely-truncated-length-classification) | Provider overflow and prematurely truncated length classification | missing | P1 | 130–250 |
| [AI-008](ai.md#ai-008---request-context-usage-checkpoints-prefix-invalidation-and-deferred-tool-estimates) | Request-context usage checkpoints, prefix invalidation and deferred tool estimates | partial | P1 | 180–320 |
| [AI-009](ai.md#ai-009---structured-assistant-diagnostics-that-preserve-nonfatal-provider-failures) | Structured assistant diagnostics that preserve nonfatal provider failures | missing | P2 | 90–180 |
| [AI-010](ai.md#ai-010---bounded-provider-error-normalization-and-response-body-preservation) | Bounded provider error normalization and response-body preservation | partial | P1 | 130–260 |
| [AI-011](ai.md#ai-011---tolerant-streaming-tool-argument-json-parsing-and-malformed-escape-policy) | Tolerant streaming tool-argument JSON parsing and malformed escape policy | partial | P1 | 150–300 |
| [AI-012](ai.md#ai-012---fail-closed-tool-schema-validation-coercion-and-accurate-argument-errors) | Fail-closed tool schema validation, coercion and accurate argument errors | partial | P0 | 220–420 |
| [AI-013](ai.md#ai-013---provider-strict-schema-and-grammar-constraints-with-explicit-requireprefer-semantics) | Provider strict-schema and grammar constraints with explicit require/prefer semantics | partial | P1 | 200–400 |
| [AI-014](ai.md#ai-014---cross-model-transcript-normalization-and-synthetic-tool-results) | Cross-model transcript normalization and synthetic tool results | partial | P2 | 140–280 |
| [AI-015](ai.md#ai-015---image-provider-collections-catalog-refresh-and-resolved-auth-generation) | Image-provider collections, catalog refresh and resolved-auth generation | partial | P2 | 240–410 |
| [AI-016](ai.md#ai-016---request-scoped-provider-environment-and-proxy-precedence) | Request-scoped provider environment and proxy precedence | partial | P1 | 130–260 |
| [AI-017](ai.md#ai-017---standalone-ai-package-oauth-cli-and-current-directory-credential-file) | Standalone AI-package OAuth CLI and current-directory credential file | different-by-design | P3 | 80–150 |
| [AI-018](ai.md#ai-018---provider-continuation-diagnostics-request-sequences-and-bounded-identifier-hashes) | Provider continuation diagnostics, request sequences and bounded identifier hashes | missing | P2 | 170–330 |
| [AI-019](ai.md#ai-019---complete-shared-message-usage-content-and-model-data-contracts) | Complete shared message, usage, content and model data contracts | partial | P1 | 140–280 |
| [AI-021](ai.md#ai-021---anthropic-request-options-thinking-modes-and-compatibility-controlled-payloads) | Anthropic request options, thinking modes and compatibility-controlled payloads | partial | P1 | 260–520 |
| [AI-022](ai.md#ai-022---anthropic-oauth-and-copilot-wire-identity-with-reversible-tool-naming) | Anthropic OAuth and Copilot wire identity with reversible tool naming | partial | P1 | 230–450 |
| [AI-023](ai.md#ai-023---anthropic-sse-terminal-integrity-live-usage-and-lossless-delta-delivery) | Anthropic SSE terminal integrity, live usage and lossless delta delivery | partial | P1 | 220–440 |
| [AI-024](ai.md#ai-024---anthropic-history-encoding-deferred-tool-placement-and-cache-marker-fidelity) | Anthropic history encoding, deferred tool placement and cache marker fidelity | partial | P1 | 240–460 |
| [AI-025](ai.md#ai-025---chat-completions-request-compatibility-thinking-formats-and-tool-controls) | Chat Completions request compatibility, thinking formats and tool controls | partial | P1 | 650–1100 |
| [AI-026](ai.md#ai-026---responses-request-controls-cache-affinity-service-tier-cost-and-stale-continuation-recovery) | Responses request controls, cache affinity, service-tier cost and stale-continuation recovery | partial | P1 | 380–620 |
| [AI-027](ai.md#ai-027---codex-account-bound-responses-request-and-cache-affinity-dialect) | Codex account-bound Responses request and cache-affinity dialect | missing | P1 | 350–580 |
| [AI-028](ai.md#ai-028---azure-responses-endpoint-deployment-and-api-version-resolution) | Azure Responses endpoint, deployment and API-version resolution | missing | P1 | 220–380 |
| [AI-029](ai.md#ai-029---ice-messages-gateway-protocol-rewrite-diagnostics-and-neutral-event-reconstruction) | ICE messages gateway protocol, rewrite diagnostics and neutral event reconstruction | missing | P1 | 280–470 |
| [AI-030](ai.md#ai-030---bedrock-converse-transport-endpoint-precedence-and-signed-request-headers) | Bedrock Converse transport, endpoint precedence and signed request headers | missing | P1 | 330–560 |
| [AI-031](ai.md#ai-031---bedrock-claude-reasoning-payloads-and-bounded-output-allowance) | Bedrock Claude reasoning payloads and bounded output allowance | missing | P1 | 190–320 |
| [AI-032](ai.md#ai-032---bedrock-history-projection-multimodal-tool-results-and-cache-points) | Bedrock history projection, multimodal tool results and cache points | missing | P1 | 280–460 |
| [AI-033](ai.md#ai-033---bedrock-event-stream-deltas-completion-integrity-and-failure-metadata) | Bedrock event-stream deltas, completion integrity and failure metadata | missing | P1 | 260–440 |
| [AI-034](ai.md#ai-034---google-generative-ai-request-transport-hooks-and-retry-envelope) | Google Generative AI request transport, hooks and retry envelope | missing | P1 | 190–320 |
| [AI-035](ai.md#ai-035---vertex-api-key-versus-adc-client-selection-and-custom-endpoint-scope) | Vertex API-key versus ADC client selection and custom endpoint scope | missing | P1 | 200–340 |
| [AI-036](ai.md#ai-036---gemini-and-vertex-thinking-levels-disable-policy-and-token-budgets) | Gemini and Vertex thinking levels, disable policy and token budgets | missing | P1 | 170–270 |
| [AI-037](ai.md#ai-037---google-history-signatures-function-ids-and-multimodal-response-grouping) | Google history signatures, function IDs and multimodal response grouping | missing | P1 | 250–410 |
| [AI-038](ai.md#ai-038---google-function-schemas-validated-mode-and-legacy-openapi-conversion) | Google function schemas, validated mode and legacy OpenAPI conversion | missing | P1 | 100–180 |
| [AI-039](ai.md#ai-039---google-and-vertex-stream-block-assembly-usage-and-finish-reason-precedence) | Google and Vertex stream block assembly, usage and finish-reason precedence | missing | P1 | 270–450 |
| [AI-040](ai.md#ai-040---mistral-chat-stream-transport-reasoning-controls-and-cache-affinity) | Mistral chat-stream transport, reasoning controls and cache affinity | missing | P1 | 210–350 |
| [AI-041](ai.md#ai-041---mistral-nine-character-tool-identity-history-and-symbol-free-schemas) | Mistral nine-character tool identity, history and symbol-free schemas | missing | P1 | 210–350 |
| [AI-042](ai.md#ai-042---mistral-streamed-tool-reconstruction-cache-usage-and-error-completion) | Mistral streamed tool reconstruction, cache usage and error completion | missing | P1 | 260–430 |
| [AI-050](ai.md#ai-050---chat-completions-prompt-cache-session-affinity-and-anthropic-style-cache-markers) | Chat Completions prompt cache, session affinity and Anthropic-style cache markers | missing | P1 | 180–310 |
| [AI-051](ai.md#ai-051---chat-completions-history-repair-reasoning-replay-and-kimi-deferred-tool-placement) | Chat Completions history repair, reasoning replay and Kimi deferred tool placement | partial | P1 | 330–550 |
| [AI-052](ai.md#ai-052---chat-completions-independent-stream-blocks-stable-tool-identities-and-diagnostic-metadata) | Chat Completions independent stream blocks, stable tool identities and diagnostic metadata | partial | P1 | 250–440 |
| [AI-053](ai.md#ai-053---responses-stream-finalization-incomplete-reasons-and-per-output-delta-fidelity) | Responses stream finalization, incomplete reasons and per-output delta fidelity | partial | P1 | 230–400 |
| [AI-054](ai.md#ai-054---responses-transcript-identities-reasoning-signatures-and-deferred-tool-search-replay) | Responses transcript identities, reasoning signatures and deferred tool-search replay | partial | P1 | 220–380 |
| [AI-055](ai.md#ai-055---codex-websocket-pooling-continuation-deltas-and-controlled-sse-fallback) | Codex WebSocket pooling, continuation deltas and controlled SSE fallback | missing | P1 | 620–1050 |
| [AI-056](ai.md#ai-056---codex-compressed-sse-terminal-framing-and-http-retry-boundaries) | Codex compressed SSE, terminal framing and HTTP retry boundaries | missing | P1 | 260–440 |
| [AI-057](ai.md#ai-057---openrouter-image-request-hooks-retries-cancellation-and-data-url-validation) | OpenRouter image request hooks, retries, cancellation and data-URL validation | partial | P2 | 150–270 |
| [AI-058](ai.md#ai-058---reproducible-model-catalogs-schema-manifests-and-staged-generation) | Reproducible model catalogs, schema manifests and staged generation | missing | P2 | 500–900 |

## AGENT

| ID | Contract | Status | Priority | Estimated Rust lines |
|---|---|---|---|---:|
| [AGENT-001](agent.md#agent-001---open-message-union-and-complete-agent-event-payloads) | Open message union and complete agent event payloads | different-by-design | P1 | 130–230 |
| [AGENT-002](agent.md#agent-002---asynchronous-stream-function-construction) | Asynchronous stream-function construction | partial | P1 | 45–85 |
| [AGENT-003](agent.md#agent-003---replaceable-default-stream-registry-and-package-entry-points) | Replaceable default stream registry and package entry points | partial | P2 | 30–65 |
| [AGENT-004](agent.md#agent-004---raw-loop-entry-continuation-sink-and-return-boundaries) | Raw loop entry, continuation, sink and return boundaries | different-by-design | P1 | 120–220 |
| [AGENT-005](agent.md#agent-005---turn-progression-and-exact-steeringfollow-up-drain-order) | Turn progression and exact steering/follow-up drain order | partial | P1 | 100–180 |
| [AGENT-006](agent.md#agent-006---context-transformation-before-provider-message-conversion) | Context transformation before provider-message conversion | partial | P1 | 55–105 |
| [AGENT-007](agent.md#agent-007---per-request-dynamic-key-and-full-stream-option-forwarding) | Per-request dynamic key and full stream-option forwarding | partial | P1 | 95–160 |
| [AGENT-008](agent.md#agent-008---streaming-assistant-reduction-and-terminal-result-selection) | Streaming assistant reduction and terminal result selection | partial | P1 | 90–160 |
| [AGENT-009](agent.md#agent-009---terminal-error-to-aborted-reclassification) | Terminal error-to-aborted reclassification | missing | P1 | 12–30 |
| [AGENT-010](agent.md#agent-010---whole-batch-sequential-override-and-truncated-call-rejection) | Whole-batch sequential override and truncated-call rejection | partial | P1 | 45–90 |
| [AGENT-011](agent.md#agent-011---parallel-completion-events-during-later-permission-preparation) | Parallel completion events during later permission preparation | partial | P0 | 130–220 |
| [AGENT-012](agent.md#agent-012---argument-preparation-validation-and-trusted-before-tool-hook) | Argument preparation, validation and trusted before-tool hook | partial | P0 | 90–160 |
| [AGENT-013](agent.md#agent-013---final-cancellation-check-immediately-before-tool-execution) | Final cancellation check immediately before tool execution | partial | P0 | 15–35 |
| [AGENT-014](agent.md#agent-014---tool-update-acknowledgement-late-update-suppression-and-raw-arguments) | Tool update acknowledgement, late-update suppression and raw arguments | partial | P0 | 110–200 |
| [AGENT-015](agent.md#agent-015---after-tool-override-and-exception-conversion) | After-tool override and exception conversion | partial | P0 | 75–140 |
| [AGENT-016](agent.md#agent-016---tool-result-normalization-empty-content-and-additional-tool-discovery) | Tool-result normalization, empty content and additional tool discovery | partial | P1 | 50–95 |
| [AGENT-017](agent.md#agent-017---all-results-termination-vote) | All-results termination vote | partial | P1 | 30–60 |
| [AGENT-018](agent.md#agent-018---atomic-next-turn-context-model-and-thinking-replacement) | Atomic next-turn context, model and thinking replacement | partial | P0 | 65–125 |
| [AGENT-019](agent.md#agent-019---stop-after-turn-gate-before-queue-consumption) | Stop-after-turn gate before queue consumption | partial | P1 | 35–75 |
| [AGENT-020](agent.md#agent-020---mutable-agent-state-snapshots-and-reducer-visibility) | Mutable Agent state, snapshots and reducer visibility | different-by-design | P1 | 95–175 |
| [AGENT-021](agent.md#agent-021---awaited-agent-subscribers-and-the-true-idle-boundary) | Awaited Agent subscribers and the true idle boundary | partial | P0 | 130–230 |
| [AGENT-022](agent.md#agent-022---steering-and-follow-up-queue-crud-and-delivery-modes) | Steering and follow-up queue CRUD and delivery modes | partial | P1 | 105–185 |
| [AGENT-023](agent.md#agent-023---continuation-from-an-assistant-tail-using-queued-work) | Continuation from an assistant tail using queued work | partial | P1 | 65–115 |
| [AGENT-024](agent.md#agent-024---synthetic-failure-lifecycle-and-guaranteed-agent-finalization) | Synthetic failure lifecycle and guaranteed Agent finalization | partial | P0 | 105–185 |
| [AGENT-025](agent.md#agent-025---agent-option-and-mutable-hook-forwarding) | Agent option and mutable hook forwarding | partial | P1 | 150–260 |
| [AGENT-026](agent.md#agent-026---prompt-overloads-and-content-shape-preservation) | Prompt overloads and content-shape preservation | partial | P2 | 25–55 |
| [AGENT-027](agent.md#agent-027---abort-wait-for-idle-and-reset-lifetime-rules) | Abort, wait-for-idle and reset lifetime rules | partial | P0 | 95–175 |
| [AGENT-028](agent.md#agent-028---bearer-authenticated-compact-event-proxy-request) | Bearer-authenticated compact-event proxy request | missing | P1 | 90–150 |
| [AGENT-029](agent.md#agent-029---compact-proxy-line-framing-and-terminal-eof-handling) | Compact proxy line framing and terminal EOF handling | missing | P1 | 95–165 |
| [AGENT-030](agent.md#agent-030---compact-proxy-content-block-reconstruction-and-signatures) | Compact proxy content-block reconstruction and signatures | missing | P1 | 170–280 |
| [AGENT-031](agent.md#agent-031---proxy-failure-conversion-cancellation-and-reader-cleanup) | Proxy failure conversion, cancellation and reader cleanup | missing | P1 | 80–140 |
| [AGENT-032](agent.md#agent-032---agentharness-v2-operations-are-explicit-scaffold-rejections) | AgentHarness v2 operations are explicit scaffold rejections | different-by-design | P3 | 0–0 |
| [AGENT-033](agent.md#agent-033---dormant-v2-hook-action-result-and-trace-type-contracts) | Dormant v2 hook, action, result and trace type contracts | different-by-design | P3 | 0–80 |
| [AGENT-034](agent.md#agent-034---scaffold-configuration-copies-and-unchecked-setters) | Scaffold configuration copies and unchecked setters | different-by-design | P2 | 30–90 |
| [AGENT-035](agent.md#agent-035---scaffold-lane-lookup-static-watches-and-close-flag) | Scaffold lane lookup, static watches and close flag | different-by-design | P3 | 0–50 |
| [AGENT-036](agent.md#agent-036---tagged-errors-and-explicit-result-adapters) | Tagged errors and explicit Result adapters | different-by-design | P2 | 45–90 |
| [AGENT-037](agent.md#agent-037---execution-environment-interfaces-and-error-classification) | Execution environment interfaces and error classification | partial | P1 | 80–140 |
| [AGENT-038](agent.md#agent-038---homefile-url-path-normalization-and-join-semantics) | Home/file-URL path normalization and join semantics | partial | P1 | 90–150 |
| [AGENT-039](agent.md#agent-039---full-utf-8-text-reads-and-malformed-byte-policy) | Full UTF-8 text reads and malformed-byte policy | partial | P1 | 15–40 |
| [AGENT-040](agent.md#agent-040---incremental-text-line-reads-and-exact-limit-behavior) | Incremental text-line reads and exact limit behavior | partial | P1 | 75–135 |
| [AGENT-041](agent.md#agent-041---binary-read-and-overwrite-cancellation-boundaries) | Binary read and overwrite cancellation boundaries | partial | P2 | 20–50 |
| [AGENT-042](agent.md#agent-042---append-parent-errors-and-ignored-typescript-cancellation) | Append parent errors and ignored TypeScript cancellation | different-by-design | P2 | 20–45 |
| [AGENT-043](agent.md#agent-043---non-following-metadata-fractional-timestamps-and-root-basename) | Non-following metadata, fractional timestamps and root basename | partial | P2 | 25–55 |
| [AGENT-044](agent.md#agent-044---directory-listing-order-special-file-skipping-and-stat-failures) | Directory listing order, special-file skipping and stat failures | partial | P2 | 20–45 |
| [AGENT-045](agent.md#agent-045---canonical-paths-and-existence-of-unsupported-objects) | Canonical paths and existence of unsupported objects | partial | P2 | 20–45 |
| [AGENT-046](agent.md#agent-046---directory-creation-and-filedirectory-removal-semantics) | Directory creation and file/directory removal semantics | partial | P1 | 45–85 |
| [AGENT-047](agent.md#agent-047---temporary-directoryfile-naming-and-lifetime) | Temporary directory/file naming and lifetime | partial | P2 | 40–80 |
| [AGENT-048](agent.md#agent-048---shell-discovery-posix-fallback-and-legacy-wsl-command-transport) | Shell discovery, POSIX fallback and legacy WSL command transport | partial | P2 | 55–105 |
| [AGENT-049](agent.md#agent-049---shell-environment-replacement-and-host-private-variable-handling) | Shell environment replacement and host-private variable handling | partial | P0 | 25–55 |
| [AGENT-050](agent.md#agent-050---shell-spawn-live-pipe-draining-and-process-registration) | Shell spawn, live pipe draining and process registration | partial | P0 | 150–260 |
| [AGENT-051](agent.md#agent-051---incremental-utf-8-output-and-callback-failure-precedence) | Incremental UTF-8 output and callback failure precedence | partial | P0 | 100–180 |
| [AGENT-052](agent.md#agent-052---timeoutabort-validation-termination-and-final-error-ordering) | Timeout/abort validation, termination and final error ordering | partial | P0 | 85–155 |
| [AGENT-053](agent.md#agent-053---post-exit-pipe-idle-grace-rather-than-a-fixed-drain-deadline) | Post-exit pipe idle grace rather than a fixed drain deadline | partial | P1 | 75–130 |
| [AGENT-054](agent.md#agent-054---best-effort-process-cleanup-without-temporary-file-ownership) | Best-effort process cleanup without temporary-file ownership | different-by-design | P1 | 50–100 |
| [AGENT-055](agent.md#agent-055---prompt-template-path-discovery-symlinks-and-ordering) | Prompt-template path discovery, symlinks and ordering | partial | P1 | 60–110 |
| [AGENT-056](agent.md#agent-056---frontmatter-fences-strict-yaml-and-template-description-fallback) | Frontmatter fences, strict YAML and template description fallback | partial | P1 | 70–130 |
| [AGENT-057](agent.md#agent-057---prompt-argument-lexer-and-ordered-javascript-substitution) | Prompt argument lexer and ordered JavaScript substitution | partial | P0 | 100–180 |
| [AGENT-058](agent.md#agent-058---source-tagged-template-mapper-and-flattened-diagnostics) | Source-tagged template mapper and flattened diagnostics | partial | P2 | 45–85 |
| [AGENT-059](agent.md#agent-059---skill-discovery-precedence-logical-symlinks-and-recursive-scope) | Skill discovery precedence, logical symlinks and recursive scope | partial | P1 | 100–175 |
| [AGENT-060](agent.md#agent-060---skill-ignore-file-prefixing-and-full-matcher-dependency) | Skill ignore-file prefixing and full matcher dependency | partial | P1 | 130–230 |
| [AGENT-061](agent.md#agent-061---skill-metadata-limits-and-preserved-invocationlist-formatting) | Skill metadata limits and preserved invocation/list formatting | partial | P1 | 60–115 |
| [AGENT-062](agent.md#agent-062---source-tagged-skill-enrichment-and-diagnostics) | Source-tagged skill enrichment and diagnostics | partial | P2 | 40–80 |
| [AGENT-063](agent.md#agent-063---bash-execution-custom-message-shape-and-text-projection) | Bash-execution custom message shape and text projection | partial | P1 | 45–90 |
| [AGENT-064](agent.md#agent-064---custom-and-summary-constructors-timestamps-and-envelopes) | Custom and summary constructors, timestamps and envelopes | partial | P1 | 75–135 |
| [AGENT-065](agent.md#agent-065---custom-role-provider-projection-and-rust-only-orphan-repair) | Custom-role provider projection and Rust-only orphan repair | different-by-design | P1 | 70–135 |

## TOOLS

| ID | Contract | Status | Priority | Estimated Rust lines |
|---|---|---|---|---:|
| [TOOLS-001](tools.md#tools-001---bash-invocation-schema-zero-timeout-normalization-and-hard-ceiling) | Bash invocation schema, zero-timeout normalization and hard ceiling | partial | P0 | 100–180 |
| [TOOLS-002](tools.md#tools-002---bash-spawn-rewriting-and-explicit-session-environment-exposure) | Bash spawn rewriting and explicit session environment exposure | partial | P1 | 100–180 |
| [TOOLS-003](tools.md#tools-003---immediate-background-launch-with-independent-process-cancellation-ownership) | Immediate background launch with independent process cancellation ownership | missing | P0 | 220–360 |
| [TOOLS-004](tools.md#tools-004---foreground-release-by-user-queued-steering-or-elapsed-time-threshold) | Foreground release by user, queued steering or elapsed-time threshold | missing | P0 | 140–230 |
| [TOOLS-005](tools.md#tools-005---background-shell-snapshots-bounded-retention-and-completion-state-machine) | Background shell snapshots, bounded retention and completion state machine | missing | P0 | 260–420 |
| [TOOLS-006](tools.md#tools-006---incremental-bash_output-reads-and-steering-interruptible-waits) | Incremental bash_output reads and steering-interruptible waits | missing | P0 | 180–280 |
| [TOOLS-007](tools.md#tools-007---bounded-background-log-tail-and-grep-without-consuming-incremental-output) | Bounded background-log tail and grep without consuming incremental output | missing | P1 | 170–280 |
| [TOOLS-008](tools.md#tools-008---stopping-running-shells-lingering-process-groups-and-session-disposal) | Stopping running shells, lingering process groups and session disposal | missing | P0 | 140–240 |
| [TOOLS-009](tools.md#tools-009---approved-byte-bounded-stdin-for-a-running-background-shell) | Approved, byte-bounded stdin for a running background shell | missing | P0 | 180–290 |
| [TOOLS-010](tools.md#tools-010---raw-output-accumulation-bounded-live-log-spill-and-trustworthy-full-output-paths) | Raw output accumulation, bounded live log spill and trustworthy full-output paths | partial | P0 | 260–420 |
| [TOOLS-011](tools.md#tools-011---session-local-bash_watch-management-and-exact-watch-schemas) | Session-local bash_watch management and exact watch schemas | missing | P0 | 180–290 |
| [TOOLS-012](tools.md#tools-012---literal-pattern-watches-with-cross-chunk-matching-and-rearm-semantics) | Literal pattern watches with cross-chunk matching and rearm semantics | missing | P1 | 70–120 |
| [TOOLS-013](tools.md#tools-013---silence-prompt-and-interval-watches-with-automatic-defaults) | Silence, prompt and interval watches with automatic defaults | missing | P0 | 180–290 |
| [TOOLS-014](tools.md#tools-014---bash-streaming-updates-elapsed-time-rendering-and-final-result-formatting) | Bash streaming updates, elapsed-time rendering and final-result formatting | partial | P1 | 140–230 |
| [TOOLS-015](tools.md#tools-015---harness-shell-capture-sanitation-early-spill-and-failure-ordering) | Harness shell capture sanitation, early spill and failure ordering | partial | P0 | 160–260 |
| [TOOLS-016](tools.md#tools-016---user-bash-execution-helper-and-its-distinct-output-contract) | User Bash execution helper and its distinct output contract | partial | P1 | 140–240 |
| [TOOLS-017](tools.md#tools-017---direct-argv-execution-with-capture-cancellation-and-timeout-result) | Direct argv execution with capture, cancellation and timeout result | partial | P1 | 130–220 |
| [TOOLS-018](tools.md#tools-018---tool-factories-definition-adapters-and-execution-context) | Tool factories, definition adapters and execution context | partial | P2 | 100–170 |
| [TOOLS-019](tools.md#tools-019---exact-truncation-metadata-utf-8-boundaries-and-line-numbering) | Exact truncation metadata, UTF-8 boundaries and line numbering | partial | P0 | 80–140 |
| [TOOLS-020](tools.md#tools-020---tool-path-normalization-and-ordered-read-path-fallback) | Tool path normalization and ordered read-path fallback | partial | P1 | 70–130 |
| [TOOLS-021](tools.md#tools-021---canonical-path-mutation-queue-admission-order-and-alias-ownership) | Canonical-path mutation queue, admission order and alias ownership | partial | P0 | 140–240 |
| [TOOLS-022](tools.md#tools-022---read-tool-pagination-cancellation-and-model-aware-image-handoff) | Read-tool pagination, cancellation and model-aware image handoff | partial | P1 | 110–190 |
| [TOOLS-023](tools.md#tools-023---image-signatures-conversion-resizing-and-coordinate-hints) | Image signatures, conversion, resizing and coordinate hints | partial | P1 | 180–310 |
| [TOOLS-024](tools.md#tools-024---write-tool-io-ordering-abort-bracketing-and-result-text) | Write-tool I/O ordering, abort bracketing and result text | partial | P2 | 40–90 |
| [TOOLS-025](tools.md#tools-025---atomic-edit-batches-with-exact-match-precedence-and-localized-fuzzy-normalization) | Atomic edit batches with exact-match precedence and localized fuzzy normalization | partial | P0 | 180–300 |
| [TOOLS-026](tools.md#tools-026---trustworthy-edit-diffs-and-asynchronous-preflight-preview) | Trustworthy edit diffs and asynchronous preflight preview | partial | P0 | 180–310 |
| [TOOLS-027](tools.md#tools-027---per-tool-call-and-result-renderers-with-streaming-content-previews) | Per-tool call and result renderers with streaming content previews | partial | P2 | 260–430 |
| [TOOLS-028](tools.md#tools-028---find-glob-semantics-hierarchical-ignore-rules-and-result-boundaries) | Find glob semantics, hierarchical ignore rules and result boundaries | partial | P1 | 220–380 |
| [TOOLS-029](tools.md#tools-029---grep-match-stream-context-formatting-and-errorcancellation-semantics) | Grep match stream, context formatting and error/cancellation semantics | partial | P1 | 230–400 |
| [TOOLS-030](tools.md#tools-030---directory-listing-symlinks-collation-and-per-entry-failure-handling) | Directory listing symlinks, collation and per-entry failure handling | partial | P1 | 90–170 |
| [TOOLS-031](tools.md#tools-031---stdout-isolation-queued-utf-8-byte-limits-and-transient-sink-retries) | Stdout isolation, queued UTF-8 byte limits and transient sink retries | partial | P1 | 160–280 |

## SESSION

| ID | Contract | Status | Priority | Estimated Rust lines |
|---|---|---|---|---:|
| [SESSION-001](session.md#session-001---directional-branch-bounds-and-cursor-semantics) | Directional branch bounds and cursor semantics | partial | P1 | 80–180 |
| [SESSION-002](session.md#session-002---fork-accounting-target-errors-and-exact-mutation-history) | Fork accounting, target errors and exact mutation history | partial | P1 | 120–300 |
| [SESSION-003](session.md#session-003---stable-insertion-order-of-in-memory-sessions-lanes-and-labels) | Stable insertion order of in-memory sessions, lanes and labels | partial | P3 | 40–110 |
| [SESSION-004](session.md#session-004---raw-entry-session-scanning-and-unicode-safe-search-results) | Raw-entry session scanning and Unicode-safe search results | partial | P1 | 140–300 |
| [SESSION-005](session.md#session-005---sqlite-session-metadata-and-repository-lifecycle) | SQLite session metadata and repository lifecycle | partial | P2 | 180–380 |
| [SESSION-006](session.md#session-006---fenced-sqlite-writer-ownership-and-atomic-session-mutations) | Fenced SQLite writer ownership and atomic session mutations | partial | P1 | 350–750 |
| [SESSION-007](session.md#session-007---relational-canonical-sqlite-layout-and-versioned-migration-contract) | Relational canonical SQLite layout and versioned migration contract | different-by-design | P2 | 650–1300 |
| [SESSION-008](session.md#session-008---derived-sqlite-branch-cache-bounded-reads-and-explicit-repair) | Derived SQLite branch cache, bounded reads and explicit repair | different-by-design | P2 | 230–500 |
| [SESSION-009](session.md#session-009---co-located-sqlite-fts5-session-search-and-transactional-indexing) | Co-located SQLite FTS5 session search and transactional indexing | missing | P2 | 170–350 |
| [SESSION-010](session.md#session-010---unicode-consistent-compaction-token-estimates-and-conversation-serialization) | Unicode-consistent compaction token estimates and conversation serialization | partial | P1 | 100–230 |
| [SESSION-011](session.md#session-011---harness-summarization-routing-retry-callbacks-and-exact-result-semantics) | Harness summarization routing, retry callbacks and exact result semantics | partial | P1 | 150–330 |
| [SESSION-012](session.md#session-012---branch-summary-collection-token-budget-grace-and-option-edge-cases) | Branch-summary collection, token-budget grace and option edge cases | partial | P2 | 90–220 |
| [SESSION-013](session.md#session-013---coding-agent-compaction-thresholds-and-reference-based-retained-context) | Coding-agent compaction thresholds and reference-based retained context | partial | P1 | 200–420 |
| [SESSION-014](session.md#session-014---coding-agent-summary-completion-validation-and-injected-stream-contract) | Coding-agent summary completion validation and injected stream contract | missing | P1 | 120–260 |
| [SESSION-015](session.md#session-015---automatic-compaction-decision-order-overflow-recovery-and-turn-boundary-mode) | Automatic compaction decision order, overflow recovery and turn-boundary mode | partial | P1 | 220–450 |
| [SESSION-016](session.md#session-016---manual-compaction-ownership-cancellation-and-lifecycle-events) | Manual compaction ownership, cancellation and lifecycle events | partial | P1 | 230–480 |
| [SESSION-017](session.md#session-017---automatic-compaction-commit-boundary-and-extension-cancellation-semantics) | Automatic compaction commit boundary and extension cancellation semantics | partial | P1 | 170–360 |
| [SESSION-018](session.md#session-018---session-retry-admission-generation-local-effect-guard-and-retry-events) | Session retry admission, generation-local effect guard and retry events | partial | P1 | 170–350 |
| [SESSION-019](session.md#session-019---prompt-preflight-input-transformation-and-busy-session-routing) | Prompt preflight, input transformation and busy-session routing | partial | P1 | 260–530 |
| [SESSION-020](session.md#session-020---custom-message-delivery-modes-and-identity-preserving-queue-controls) | Custom-message delivery modes and identity-preserving queue controls | partial | P1 | 210–450 |
| [SESSION-021](session.md#session-021---runtime-session-replacement-teardown-settlement-and-exclusive-import) | Runtime session replacement, teardown settlement and exclusive import | partial | P1 | 330–670 |
| [SESSION-022](session.md#session-022---coding-session-event-ordering-durable-writes-and-final-settlement) | Coding-session event ordering, durable writes and final settlement | partial | P1 | 280–560 |
| [SESSION-023](session.md#session-023---session-owned-tool-registry-activation-and-background-control-integration) | Session-owned tool registry, activation and background-control integration | partial | P1 | 220–450 |
| [SESSION-024](session.md#session-024---user-bash-operation-ownership-streaming-updates-and-ordered-persistence) | User-bash operation ownership, streaming updates and ordered persistence | partial | P1 | 190–390 |
| [SESSION-025](session.md#session-025---background-shell-completionwatch-delivery-idle-wake-and-disposal-wiring) | Background-shell completion/watch delivery, idle wake and disposal wiring | missing | P1 | 180–380 |
| [SESSION-026](session.md#session-026---tree-navigation-with-branch-summarization-veto-and-commit-aware-cancellation) | Tree navigation with branch summarization, veto and commit-aware cancellation | partial | P1 | 240–500 |
| [SESSION-027](session.md#session-027---all-history-usage-totals-and-post-compaction-context-uncertainty) | All-history usage totals and post-compaction context uncertainty | partial | P2 | 180–370 |
| [SESSION-028](session.md#session-028---sdk-construction-cwd-bound-services-and-live-request-configuration) | SDK construction, cwd-bound services and live request configuration | partial | P1 | 320–650 |
| [SESSION-029](session.md#session-029---coding-session-version-3-schema-identity-and-legacy-migrations) | Coding-session version 3 schema, identity and legacy migrations | different-by-design | P1 | 420–850 |
| [SESSION-030](session.md#session-030---bounded-tolerant-session-reads-versus-native-log-recovery) | Bounded tolerant session reads versus native log recovery | different-by-design | P1 | 280–580 |
| [SESSION-031](session.md#session-031---lazy-first-message-persistence-exclusive-creation-and-durable-append-barrier) | Lazy first-message persistence, exclusive creation and durable append barrier | different-by-design | P1 | 230–470 |
| [SESSION-032](session.md#session-032---session-discovery-activity-timestamps-cwd-scoping-and-deletion-fallback) | Session discovery, activity timestamps, cwd scoping and deletion fallback | partial | P2 | 200–430 |
| [SESSION-033](session.md#session-033---coding-branch-extraction-label-history-and-parent-path-provenance) | Coding branch extraction, label history and parent-path provenance | different-by-design | P1 | 230–470 |
| [SESSION-034](session.md#session-034---coding-sessionmanager-read-views-context-projection-and-local-mutation-events) | Coding SessionManager read views, context projection and local mutation events | different-by-design | P2 | 200–430 |
| [SESSION-035](session.md#session-035---extension-binding-resource-extension-and-reload-sequencing) | Extension binding, resource extension and reload sequencing | partial | P1 | 240–510 |
| [SESSION-036](session.md#session-036---modelthinking-selection-metadata-refresh-and-settings-synchronization) | Model/thinking selection, metadata refresh and settings synchronization | partial | P1 | 190–400 |
| [SESSION-037](session.md#session-037---context-occupancy-breakdown-and-tool-source-attribution) | Context occupancy breakdown and tool-source attribution | missing | P2 | 210–420 |
| [SESSION-038](session.md#session-038---active-branch-jsonl-export-and-current-context-assistant-text) | Active-branch JSONL export and current-context assistant text | partial | P2 | 140–300 |

## SET

| ID | Contract | Status | Priority | Estimated Rust lines |
|---|---|---|---|---:|
| [SET-001](settings.md#set-001---distribution-identity-agent-roots-and-bundled-resource-path-resolution) | Distribution identity, agent roots and bundled resource path resolution | partial | P1 | 180–320 |
| [SET-002](settings.md#set-002---install-method-detection-and-safe-self-update-command-selection) | Install-method detection and safe self-update command selection | different-by-design | P2 | 80–180 |
| [SET-003](settings.md#set-003---project-trust-resolution-before-loading-executable-resources) | Project trust resolution before loading executable resources | partial | P0 | 280–480 |
| [SET-004](settings.md#set-004---locked-trust-store-with-canonical-ancestor-inheritance-and-grouped-updates) | Locked trust store with canonical ancestor inheritance and grouped updates | partial | P0 | 170–280 |
| [SET-005](settings.md#set-005---configuration-environment-templates-introspection-and-strict-header-resolution) | Configuration environment templates, introspection and strict header resolution | partial | P1 | 150–260 |
| [SET-006](settings.md#set-006---bounded-command-backed-configuration-values-and-cached-failures) | Bounded command-backed configuration values and cached failures | partial | P1 | 140–240 |
| [SET-007](settings.md#set-007---settings-storage-abstraction-lock-discipline-and-parse-failure-preservation) | Settings storage abstraction, lock discipline and parse-failure preservation | partial | P1 | 260–420 |
| [SET-008](settings.md#set-008---scoped-deep-precedence-launcher-global-first-policy-and-change-notifications) | Scoped deep precedence, launcher global-first policy and change notifications | partial | P0 | 260–430 |
| [SET-009](settings.md#set-009---queued-changed-field-settings-writes-nested-merges-and-scoped-mutation-apis) | Queued changed-field settings writes, nested merges and scoped mutation APIs | partial | P1 | 240–420 |
| [SET-010](settings.md#set-010---legacy-settings-migration-without-implicit-product-aliases) | Legacy settings migration without implicit product aliases | partial | P2 | 90–150 |
| [SET-011](settings.md#set-011---model-defaults-queue-modes-transport-preferences-and-global-favorites) | Model defaults, queue modes, transport preferences and global favorites | partial | P1 | 160–280 |
| [SET-012](settings.md#set-012---compaction-branch-summary-and-two-layer-retry-settings) | Compaction, branch-summary and two-layer retry settings | partial | P1 | 130–240 |
| [SET-013](settings.md#set-013---http-proxy-application-active-request-idle-deadlines-and-dispatcher-replacement) | HTTP proxy application, active-request idle deadlines and dispatcher replacement | partial | P1 | 180–320 |
| [SET-014](settings.md#set-014---shell-path-background-timers-and-restrictive-stdin-approval-composition) | Shell path, background timers and restrictive stdin approval composition | partial | P0 | 120–220 |
| [SET-015](settings.md#set-015---display-editor-cursor-and-warning-preferences-with-exact-fallback-rules) | Display, editor, cursor and warning preferences with exact fallback rules | partial | P2 | 200–360 |
| [SET-016](settings.md#set-016---independent-image-transmission-resizing-and-terminal-presentation-settings) | Independent image transmission, resizing and terminal presentation settings | partial | P1 | 100–190 |
| [SET-017](settings.md#set-017---typed-fixedautomatic-themes-and-global-appearance-persistence) | Typed fixed/automatic themes and global appearance persistence | partial | P2 | 100–200 |
| [SET-018](settings.md#set-018---install-telemetry-analytics-consent-and-changelog-persistence) | Install telemetry, analytics consent and changelog persistence | partial | P2 | 60–120 |
| [SET-019](settings.md#set-019---scoped-resource-arrays-package-filters-and-skill-command-preference) | Scoped resource arrays, package filters and skill-command preference | partial | P1 | 130–240 |
| [SET-020](settings.md#set-020---scoped-icesubagents-and-icehooks-storage-without-permissive-authority-merging) | Scoped ice.subagents and ice.hooks storage without permissive authority merging | partial | P0 | 80–150 |
| [SET-021](settings.md#set-021---resource-reload-orchestration-disable-flags-and-precedence) | Resource reload orchestration, disable flags and precedence | partial | P1 | 260–440 |
| [SET-022](settings.md#set-022---trust-bootstrap-extension-reuse-without-duplicate-factory-side-effects) | Trust bootstrap extension reuse without duplicate factory side effects | partial | P0 | 170–300 |
| [SET-023](settings.md#set-023---resource-provenance-canonical-deduplication-and-extension-added-paths) | Resource provenance, canonical deduplication and extension-added paths | partial | P1 | 150–270 |
| [SET-024](settings.md#set-024---extension-resource-conflict-diagnostics-without-unloading-registrations) | Extension resource conflict diagnostics without unloading registrations | partial | P2 | 70–130 |
| [SET-025](settings.md#set-025---project-instructions-ancestor-ordering-and-linked-worktree-shadow-suppression) | Project instructions, ancestor ordering and linked-worktree shadow suppression | partial | P1 | 180–320 |
| [SET-026](settings.md#set-026---systemmd-append_systemmd-and-global-base-template-source-resolution) | SYSTEM.md, APPEND_SYSTEM.md and global base-template source resolution | partial | P1 | 160–280 |
| [SET-027](settings.md#set-027---dynamic-coding-system-prompt-tool-metadata-and-one-pass-base-placeholders) | Dynamic coding system prompt, tool metadata and one-pass base placeholders | partial | P1 | 180–310 |
| [SET-028](settings.md#set-028---coding-prompt-template-discovery-metadata-and-source-aware-collision-handling) | Coding prompt-template discovery, metadata and source-aware collision handling | partial | P2 | 170–290 |
| [SET-029](settings.md#set-029---coding-slash-template-argument-parsing-and-nonrecursive-fallback-substitution) | Coding slash-template argument parsing and nonrecursive fallback substitution | partial | P1 | 120–210 |
| [SET-030](settings.md#set-030---skill-discovery-ignore-rules-and-canonical-filename-deduplication) | Skill discovery, ignore rules and canonical-file/name deduplication | partial | P1 | 190–340 |
| [SET-031](settings.md#set-031---skill-frontmatter-validation-and-model-visible-xml-listing) | Skill frontmatter validation and model-visible XML listing | partial | P2 | 90–170 |
| [SET-032](settings.md#set-032---skill-signatures-automatic-refresh-and-promptautocomplete-propagation) | Skill signatures, automatic refresh and prompt/autocomplete propagation | partial | P1 | 180–310 |
| [SET-033](settings.md#set-033---theme-file-discovery-source-ownership-and-first-name-collision-diagnostics) | Theme-file discovery, source ownership and first-name collision diagnostics | missing | P2 | 160–280 |

## EXT

| ID | Contract | Status | Priority | Estimated Rust lines |
|---|---|---|---|---:|
| [EXT-001](extensions.md#ext-001---typescript-factory-execution-versus-the-native-abi-boundary) | TypeScript factory execution versus the native ABI boundary | different-by-design | P0 | 100–180 |
| [EXT-002](extensions.md#ext-002---exact-historical-module-aliases-and-runtime-specific-import-resolution) | Exact historical module aliases and runtime-specific import resolution | different-by-design | P2 | 0–80 |
| [EXT-003](extensions.md#ext-003---cwdgeneration-factory-cache-with-fresh-instance-initialization) | Cwd/generation factory cache with fresh instance initialization | partial | P2 | 70–130 |
| [EXT-004](extensions.md#ext-004---ordered-discovery-and-lexical-path-deduplication) | Ordered discovery and lexical path deduplication | partial | P1 | 90–160 |
| [EXT-005](extensions.md#ext-005---ice-over-pi-resource-manifest-and-entrypoint-fallback) | ICE-over-Pi resource manifest and entrypoint fallback | partial | P1 | 70–125 |
| [EXT-006](extensions.md#ext-006---factory-local-registration-staging-and-commit-ordering) | Factory-local registration staging and commit ordering | partial | P0 | 110–190 |
| [EXT-007](extensions.md#ext-007---initialization-console-buffering-and-restoration) | Initialization console buffering and restoration | missing | P2 | 90–160 |
| [EXT-008](extensions.md#ext-008---copy-on-write-event-registration-and-idempotent-unsubscribe) | Copy-on-write event registration and idempotent unsubscribe | partial | P1 | 70–125 |
| [EXT-009](extensions.md#ext-009---core-action-rebinding-and-explicit-ui-mode-defaults) | Core action rebinding and explicit UI mode defaults | partial | P1 | 120–210 |
| [EXT-010](extensions.md#ext-010---first-cause-invalidation-and-lazy-stale-context-guards) | First-cause invalidation and lazy stale-context guards | partial | P0 | 80–150 |
| [EXT-011](extensions.md#ext-011---object-schema-tool-registration-and-immediate-refresh-without-rollback) | Object-schema tool registration and immediate refresh without rollback | partial | P1 | 100–180 |
| [EXT-012](extensions.md#ext-012---fresh-context-tool-wrapper-and-additive-active-tool-attribution) | Fresh-context tool wrapper and additive active-tool attribution | partial | P1 | 55–105 |
| [EXT-013](extensions.md#ext-013---validated-command-registration-and-collision-suffix-aliases) | Validated command registration and collision suffix aliases | partial | P1 | 90–155 |
| [EXT-014](extensions.md#ext-014---reserved-shortcut-union-and-last-extension-wins-diagnostics) | Reserved shortcut union and last-extension-wins diagnostics | partial | P1 | 95–170 |
| [EXT-015](extensions.md#ext-015---flag-type-validation-staged-defaults-and-owner-only-lookup) | Flag type validation, staged defaults and owner-only lookup | partial | P1 | 65–120 |
| [EXT-016](extensions.md#ext-016---named-settings-sections-without-automatic-persistence) | Named settings sections without automatic persistence | missing | P1 | 120–210 |
| [EXT-017](extensions.md#ext-017---asynchronous-shared-event-bus-with-staged-listener-ownership) | Asynchronous shared event bus with staged listener ownership | partial | P1 | 100–180 |
| [EXT-018](extensions.md#ext-018---overloaded-provider-registration-grouped-queue-flush-and-refresh-callbacks) | Overloaded provider registration, grouped queue flush and refresh callbacks | partial | P1 | 140–250 |
| [EXT-019](extensions.md#ext-019---live-extension-context-and-host-control-getters) | Live extension context and host control getters | partial | P1 | 170–290 |
| [EXT-020](extensions.md#ext-020---command-only-session-replacement-context-and-callbacks) | Command-only session replacement context and callbacks | partial | P1 | 190–330 |
| [EXT-021](extensions.md#ext-021---error-notifications-and-post-completion-slow-handler-observation) | Error notifications and post-completion slow-handler observation | partial | P1 | 90–160 |
| [EXT-022](extensions.md#ext-022---generic-lifecycle-dispatch-and-last-returned-session-veto-payload) | Generic lifecycle dispatch and last-returned session veto payload | partial | P0 | 220–380 |
| [EXT-023](extensions.md#ext-023---final-message-replacement-with-same-role-enforcement) | Final-message replacement with same-role enforcement | partial | P1 | 100–180 |
| [EXT-024](extensions.md#ext-024---mutable-pre-tool-block-decision-with-propagated-hook-failures) | Mutable pre-tool block decision with propagated hook failures | partial | P0 | 160–270 |
| [EXT-025](extensions.md#ext-025---chained-tool-result-field-patches-and-usage-propagation) | Chained tool-result field patches and usage propagation | partial | P0 | 130–225 |
| [EXT-026](extensions.md#ext-026---user-shell-operation-replacement-and-error-propagation) | User-shell operation replacement and error propagation | partial | P1 | 100–180 |
| [EXT-027](extensions.md#ext-027---deep-cloned-model-context-transformation-chain) | Deep-cloned model-context transformation chain | partial | P1 | 120–210 |
| [EXT-028](extensions.md#ext-028---actual-provider-payload-replacement-chain) | Actual provider-payload replacement chain | partial | P0 | 130–230 |
| [EXT-029](extensions.md#ext-029---in-place-provider-header-record-mutation) | In-place provider header-record mutation | partial | P1 | 90–160 |
| [EXT-030](extensions.md#ext-030---per-turn-system-prompt-chain-plus-custom-message-injection) | Per-turn system-prompt chain plus custom-message injection | partial | P0 | 170–290 |
| [EXT-031](extensions.md#ext-031---resource-discovery-results-with-declaring-extension-attribution) | Resource discovery results with declaring-extension attribution | partial | P1 | 100–180 |
| [EXT-032](extensions.md#ext-032---submitted-input-transformation-and-streaming-delivery-metadata) | Submitted-input transformation and streaming-delivery metadata | partial | P0 | 130–220 |
| [EXT-033](extensions.md#ext-033---native-event-payload-ownership-field-loss-and-late-callback-lifetime) | Native event payload ownership, field loss and late callback lifetime | partial | P0 | 200–350 |
| [EXT-034](extensions.md#ext-034---project-trust-decision-hook-and-malformed-return-behavior) | Project-trust decision hook and malformed-return behavior | partial | P0 | 100–180 |
| [EXT-035](extensions.md#ext-035---extension-tool-metadata-argument-preparation-and-execution-policy) | Extension tool metadata, argument preparation and execution policy | partial | P1 | 160–300 |
| [EXT-036](extensions.md#ext-036---per-tool-render-callbacks-row-state-and-fallback-rendering) | Per-tool render callbacks, row state and fallback rendering | missing | P2 | 240–440 |
| [EXT-037](extensions.md#ext-037---custom-message-renderer-selection-expansion-and-fallback) | Custom message renderer selection, expansion and fallback | partial | P1 | 100–190 |
| [EXT-038](extensions.md#ext-038---persistent-custom-entry-rendering-and-hidden-by-default-records) | Persistent custom entry rendering and hidden-by-default records | partial | P1 | 80–160 |
| [EXT-039](extensions.md#ext-039---ordered-markdown-transformation-with-message-and-layout-context) | Ordered Markdown transformation with message and layout context | partial | P2 | 80–150 |
| [EXT-040](extensions.md#ext-040---extension-dialogs-cancellation-and-interactiverpc-projection) | Extension dialogs, cancellation and interactive/RPC projection | partial | P1 | 220–400 |
| [EXT-041](extensions.md#ext-041---extension-notifications-identity-and-sticky-notices) | Extension notifications, identity and sticky notices | missing | P2 | 80–150 |
| [EXT-042](extensions.md#ext-042---status-text-working-indicator-and-hidden-thinking-controls) | Status text, working indicator and hidden-thinking controls | partial | P2 | 110–210 |
| [EXT-043](extensions.md#ext-043---keyed-extension-widgets-and-headless-safe-rpc-projections) | Keyed extension widgets and headless-safe RPC projections | partial | P2 | 130–240 |
| [EXT-044](extensions.md#ext-044---custom-footer-header-and-terminal-title) | Custom footer, header and terminal title | partial | P2 | 120–230 |
| [EXT-045](extensions.md#ext-045---focused-custom-ui-components-overlays-and-completion-lifetime) | Focused custom UI components, overlays and completion lifetime | partial | P2 | 220–420 |
| [EXT-046](extensions.md#ext-046---raw-terminal-input-interception-and-data-transformation) | Raw terminal input interception and data transformation | partial | P2 | 110–210 |
| [EXT-047](extensions.md#ext-047---editor-text-bracketed-paste-and-custom-editor-replacement) | Editor text, bracketed paste and custom editor replacement | partial | P1 | 180–320 |
| [EXT-048](extensions.md#ext-048---stacked-autocomplete-provider-wrappers) | Stacked autocomplete provider wrappers | missing | P2 | 160–280 |
| [EXT-049](extensions.md#ext-049---theme-inspection-loading-and-ui-theme-changes) | Theme inspection, loading and UI theme changes | missing | P2 | 120–220 |
| [EXT-050](extensions.md#ext-050---tool-output-expansion-controls-and-batch-state-semantics) | Tool-output expansion controls and batch-state semantics | missing | P2 | 60–120 |
| [EXT-051](extensions.md#ext-051---custom-message-delivery-next-turn-deferral-and-background-errors) | Custom message delivery, next-turn deferral and background errors | partial | P1 | 100–190 |
| [EXT-052](extensions.md#ext-052---extension-user-message-textimage-normalization-and-source-routing) | Extension user-message text/image normalization and source routing | partial | P1 | 100–190 |
| [EXT-053](extensions.md#ext-053---custom-entry-append-durable-flush-and-session-metadata-actions) | Custom entry append, durable flush and session metadata actions | partial | P1 | 120–230 |
| [EXT-054](extensions.md#ext-054---tool-inventory-active-tool-selection-and-command-introspection) | Tool inventory, active-tool selection and command introspection | partial | P1 | 130–230 |
| [EXT-055](extensions.md#ext-055---extension-model-selection-thinking-levels-and-change-events) | Extension model selection, thinking levels and change events | partial | P1 | 110–210 |
| [EXT-056](extensions.md#ext-056---extension-direct-process-execution-api-and-captured-working-directory) | Extension direct-process execution API and captured working directory | missing | P2 | 130–230 |
| [EXT-057](extensions.md#ext-057---inline-extension-identity-hidden-display-and-before-user-ordering) | Inline extension identity, hidden display and before-user ordering | different-by-design | P2 | 90–180 |

## RPC

| ID | Contract | Status | Priority | Estimated Rust lines |
|---|---|---|---|---:|
| [RPC-001](rpc-remote.md#rpc-001---bounded-binary-framing-and-terminal-decoder-state) | Bounded binary framing and terminal decoder state | partial | P1 | 45–110 |
| [RPC-002](rpc-remote.md#rpc-002---strict-cbor-value-domain-allocation-limits-and-tag-depth-defect) | Strict CBOR value domain, allocation limits and tag-depth defect | partial | P1 | 150–330 |
| [RPC-003](rpc-remote.md#rpc-003---closed-version-one-binary-schemas-and-validated-codec-boundaries) | Closed version-one binary schemas and validated codec boundaries | missing | P1 | 450–850 |
| [RPC-004](rpc-remote.md#rpc-004---explicit-ai-to-protocol-transcript-and-diagnostic-projection) | Explicit AI-to-protocol transcript and diagnostic projection | missing | P1 | 180–360 |
| [RPC-005](rpc-remote.md#rpc-005---binary-server-handshake-stages-and-failure-containment) | Binary server handshake stages and failure containment | missing | P1 | 230–450 |
| [RPC-006](rpc-remote.md#rpc-006---binary-server-listener-lifecycle-and-serialized-snapshot-revisions) | Binary server listener lifecycle and serialized snapshot revisions | missing | P1 | 130–280 |
| [RPC-007](rpc-remote.md#rpc-007---shared-live-runtime-ownership-and-work-survival-after-detach) | Shared live-runtime ownership and work survival after detach | missing | P1 | 280–550 |
| [RPC-008](rpc-remote.md#rpc-008---unix-listener-publication-permissions-and-inode-safe-cleanup) | Unix listener publication, permissions and inode-safe cleanup | missing | P1 | 180–360 |
| [RPC-009](rpc-remote.md#rpc-009---bounded-ordered-unix-byte-transport-and-final-close-semantics) | Bounded ordered Unix byte transport and final close semantics | missing | P1 | 150–300 |
| [RPC-010](rpc-remote.md#rpc-010---binary-client-connection-generations-and-deterministic-disposal) | Binary client connection generations and deterministic disposal | missing | P1 | 220–440 |
| [RPC-011](rpc-remote.md#rpc-011---binary-request-correlation-and-authoritative-client-snapshots) | Binary request correlation and authoritative client snapshots | missing | P1 | 160–330 |
| [RPC-012](rpc-remote.md#rpc-012---client-sharedexclusive-session-leases-and-failed-cleanup-reconciliation) | Client shared/exclusive session leases and failed-cleanup reconciliation | missing | P1 | 180–360 |
| [RPC-013](rpc-remote.md#rpc-013---transient-transcript-projection-with-authoritative-snapshot-reset) | Transient transcript projection with authoritative snapshot reset | missing | P1 | 110–230 |
| [RPC-014](rpc-remote.md#rpc-014---borrowed-client-remotesession-operations-replacement-rollback-and-disposal) | Borrowed-client RemoteSession operations, replacement rollback and disposal | missing | P1 | 240–480 |
| [RPC-015](rpc-remote.md#rpc-015---experimental-remote-command-parsing-and-native-connect-mode-boundary) | Experimental remote command parsing and native connect-mode boundary | partial | P1 | 150–300 |
| [RPC-016](rpc-remote.md#rpc-016---jsonl-framing-command-inventory-and-pre-dispatch-validation) | JSONL framing, command inventory and pre-dispatch validation | partial | P1 | 350–650 |
| [RPC-017](rpc-remote.md#rpc-017---bounded-jsonl-output-stream-projection-and-sink-failure-boundary) | Bounded JSONL output, stream projection and sink-failure boundary | partial | P1 | 180–340 |
| [RPC-018](rpc-remote.md#rpc-018---jsonl-command-admission-mutation-ordering-and-transition-deadlock-guards) | JSONL command admission, mutation ordering and transition deadlock guards | missing | P1 | 240–430 |
| [RPC-019](rpc-remote.md#rpc-019---request-owned-cancellation-queue-time-deadlines-and-terminal-response-semantics) | Request-owned cancellation, queue-time deadlines and terminal response semantics | missing | P1 | 170–310 |
| [RPC-020](rpc-remote.md#rpc-020---jsonl-bootstrap-runtime-rebinding-shutdown-and-explicit-recovery) | JSONL bootstrap, runtime rebinding, shutdown and explicit recovery | partial | P1 | 230–440 |
| [RPC-021](rpc-remote.md#rpc-021---jsonl-prompt-admission-attachments-and-conversational-queue-controls) | JSONL prompt admission, attachments and conversational queue controls | partial | P1 | 220–420 |
| [RPC-022](rpc-remote.md#rpc-022---jsonl-capabilities-state-resource-introspection-and-invalidation-events) | JSONL capabilities, state, resource introspection and invalidation events | partial | P1 | 280–520 |
| [RPC-023](rpc-remote.md#rpc-023---jsonl-model-catalogs-explicit-refresh-and-live-session-controls) | JSONL model catalogs, explicit refresh and live session controls | partial | P1 | 250–470 |
| [RPC-024](rpc-remote.md#rpc-024---jsonl-session-listings-treemessage-pages-and-textstatistic-views) | JSONL session listings, tree/message pages and text/statistic views | missing | P1 | 230–420 |
| [RPC-025](rpc-remote.md#rpc-025---jsonl-session-replacement-branch-navigation-and-fileexport-mutations) | JSONL session replacement, branch navigation and file/export mutations | missing | P1 | 220–430 |
| [RPC-026](rpc-remote.md#rpc-026---jsonl-user-bash-and-background-shell-observationcontrol) | JSONL user Bash and background-shell observation/control | missing | P1 | 140–270 |
| [RPC-027](rpc-remote.md#rpc-027---jsonl-subagent-overview-approvals-controls-and-authorized-patch-wrapper) | JSONL subagent overview, approvals, controls and authorized patch wrapper | missing | P1 | 150–290 |
| [RPC-028](rpc-remote.md#rpc-028---jsonl-extension-dialog-bridge-and-bounded-ui-text-snapshots) | JSONL extension dialog bridge and bounded UI text snapshots | partial | P1 | 220–420 |
| [RPC-029](rpc-remote.md#rpc-029---jsonl-settings-catalog-effective-value-projection-and-diagnostics) | JSONL settings catalog, effective-value projection and diagnostics | missing | P1 | 260–480 |
| [RPC-030](rpc-remote.md#rpc-030---jsonl-settings-mutation-validation-persistence-and-live-apply-ordering) | JSONL settings mutation validation, persistence and live-apply ordering | missing | P1 | 190–360 |
| [RPC-031](rpc-remote.md#rpc-031---structured-slash-command-inventory-schemas-revisions-and-invocation) | Structured slash-command inventory, schemas, revisions and invocation | missing | P1 | 300–570 |
| [RPC-032](rpc-remote.md#rpc-032---jsonl-child-process-client-startup-handshake-and-termination) | JSONL child-process client startup, handshake and termination | missing | P1 | 180–330 |
| [RPC-033](rpc-remote.md#rpc-033---jsonl-client-request-ownership-bounded-writes-and-deadline-cancellation) | JSONL client request ownership, bounded writes and deadline cancellation | missing | P1 | 200–360 |
| [RPC-034](rpc-remote.md#rpc-034---command-specific-jsonl-response-validation-and-bounded-opaque-metadata) | Command-specific JSONL response validation and bounded opaque metadata | missing | P1 | 330–560 |
| [RPC-035](rpc-remote.md#rpc-035---jsonl-client-event-delivery-idle-waiters-and-bounded-collectors) | JSONL client event delivery, idle waiters and bounded collectors | missing | P1 | 130–230 |
| [RPC-036](rpc-remote.md#rpc-036---native-tcp-proxy-and-transcript-projection-preservation-with-premature-stream-termination) | Native TCP proxy and transcript projection preservation, with premature stream termination | different-by-design | P1 | 40–100 |

## TUI

| ID | Contract | Status | Priority | Estimated Rust lines |
|---|---|---|---|---:|
| [TUI-001](tui.md#tui-001---json-theme-format-semantic-color-tokens-and-ansi-rendering) | JSON theme format, semantic color tokens and ANSI rendering | partial | P1 | 450–750 |
| [TUI-002](tui.md#tui-002---bundled-palette-assets-and-exact-catalog-identities) | Bundled palette assets and exact catalog identities | partial | P1 | 180–320 |
| [TUI-003](tui.md#tui-003---named-theme-registry-global-live-switching-and-selector-integration) | Named theme registry, global live switching and selector integration | partial | P1 | 250–440 |
| [TUI-004](tui.md#tui-004---resolved-theme-colors-and-optional-palette-values-for-html-export) | Resolved theme colors and optional palette values for HTML export | partial | P2 | 100–180 |
| [TUI-005](tui.md#tui-005---syntax-highlighting-markdown-theme-and-interactive-chrome-adapters) | Syntax highlighting, Markdown theme and interactive chrome adapters | partial | P2 | 450–800 |
| [TUI-006](tui.md#tui-006---unicode-nerd-font-and-ascii-symbol-presets-with-locale-defaults) | Unicode, Nerd Font and ASCII symbol presets with locale defaults | missing | P2 | 140–260 |
| [TUI-007](tui.md#tui-007---terminal-background-detection-and-automatic-lightdark-theme-synchronization) | Terminal background detection and automatic light/dark theme synchronization | missing | P2 | 180–300 |
| [TUI-008](tui.md#tui-008---active-custom-theme-file-watching-debounce-and-error-retention) | Active custom-theme file watching, debounce and error retention | missing | P2 | 100–180 |
| [TUI-009](tui.md#tui-009---versioned-appearance-data-deterministic-migration-and-bounded-normalization) | Versioned appearance data, deterministic migration and bounded normalization | missing | P1 | 750–1200 |
| [TUI-010](tui.md#tui-010---appearance-color-resolution-and-textmarkdown-presentation-adapters) | Appearance color resolution and text/Markdown presentation adapters | partial | P1 | 250–450 |
| [TUI-011](tui.md#tui-011---appearance-profile-files-partial-merge-and-legacy-profile-migration) | Appearance profile files, partial merge and legacy profile migration | missing | P2 | 300–500 |
| [TUI-012](tui.md#tui-012---portable-appearance-bundles-and-importexport-validation) | Portable appearance bundles and import/export validation | missing | P2 | 250–400 |
| [TUI-013](tui.md#tui-013---custom-theme-file-library-and-immediate-palette-editing) | Custom theme file library and immediate palette editing | missing | P2 | 250–400 |
| [TUI-014](tui.md#tui-014---in-memory-theme-drafts-variable-editing-and-retryable-apply-phases) | In-memory theme drafts, variable editing and retryable apply phases | missing | P1 | 350–600 |
| [TUI-015](tui.md#tui-015---staged-profile-library-changes-and-overwrite-safe-retries) | Staged profile-library changes and overwrite-safe retries | missing | P2 | 150–280 |
| [TUI-016](tui.md#tui-016---unified-appearancetheme-customization-preview-reset-apply-and-cancel) | Unified appearance/theme customization preview, reset, apply and cancel | missing | P1 | 250–450 |
| [TUI-017](tui.md#tui-017---literal-input-highlighting-with-stable-source-offsets-and-overlap-priority) | Literal input highlighting with stable source offsets and overlap priority | missing | P2 | 180–320 |
| [TUI-018](tui.md#tui-018---thinking-verb-selection-indicator-mirroring-and-text-styling-helpers) | Thinking verb selection, indicator mirroring and text styling helpers | partial | P2 | 80–150 |
| [TUI-019](tui.md#tui-019---appearance-customizer-viewport-navigation-draft-validation-and-disposal) | Appearance customizer viewport, navigation, draft validation and disposal | missing | P1 | 420–700 |
| [TUI-020](tui.md#tui-020---appearance-field-controls-shared-color-editor-and-highlighter-rule-editor) | Appearance field controls, shared color editor and highlighter-rule editor | missing | P1 | 650–1000 |
| [TUI-021](tui.md#tui-021---customizer-theme-editor-token-groups-variables-and-staged-selection-divergence) | Customizer theme editor, token groups, variables and staged-selection divergence | missing | P1 | 450–750 |
| [TUI-022](tui.md#tui-022---customizer-profile-and-bundle-ui-scope-selection-and-partial-staging) | Customizer profile and bundle UI, scope selection and partial staging | missing | P1 | 420–700 |
| [TUI-023](tui.md#tui-023---appearance-preview-scenes-real-component-samples-and-animation-cleanup) | Appearance preview scenes, real component samples and animation cleanup | missing | P2 | 320–550 |
| [TUI-024](tui.md#tui-024---interactive-appearance-application-settings-preview-and-message-stamping) | Interactive appearance application, settings preview and message stamping | partial | P1 | 400–700 |
| [TUI-025](tui.md#tui-025---searchable-settings-menu-exact-choices-and-extension-sections) | Searchable settings menu, exact choices and extension sections | partial | P1 | 500–800 |
| [TUI-026](tui.md#tui-026---theme-thinking-and-image-choice-submenus-with-preview-rollback) | Theme, thinking and image choice submenus with preview rollback | partial | P1 | 320–520 |
| [TUI-027](tui.md#tui-027---user-message-formatting-width-budgets-and-osc-133-zones) | User message formatting, width budgets and OSC 133 zones | partial | P1 | 300–500 |
| [TUI-028](tui.md#tui-028---assistant-content-ordering-reasoning-visibility-bullets-and-display-transforms) | Assistant content ordering, reasoning visibility, bullets and display transforms | partial | P1 | 400–700 |
| [TUI-029](tui.md#tui-029---default-custom-message-cards-completion-summaries-and-renderer-precedence) | Default custom-message cards, completion summaries and renderer precedence | partial | P1 | 320–550 |
| [TUI-030](tui.md#tui-030---branch-compaction-and-skill-summary-cards) | Branch, compaction and skill summary cards | partial | P2 | 150–260 |
| [TUI-031](tui.md#tui-031---tool-renderer-composition-execution-context-images-and-live-result-binding) | Tool renderer composition, execution context, images and live result binding | partial | P1 | 500–850 |
| [TUI-032](tui.md#tui-032---compact-tool-rows-result-precedence-and-bounded-summaries) | Compact tool rows, result precedence and bounded summaries | partial | P1 | 300–500 |
| [TUI-033](tui.md#tui-033---exploration-grouping-and-display-only-shell-summaries) | Exploration grouping and display-only shell summaries | missing | P2 | 200–350 |
| [TUI-034](tui.md#tui-034---application-action-links-and-incremental-tool-expansion) | Application action links and incremental tool expansion | partial | P2 | 220–380 |
| [TUI-035](tui.md#tui-035---user-bash-streaming-display-visual-tail-and-completion-presentation) | User Bash streaming display, visual tail and completion presentation | partial | P1 | 220–380 |
| [TUI-036](tui.md#tui-036---diff-row-parsing-word-emphasis-and-appearance-sensitive-tinting) | Diff row parsing, word emphasis and appearance-sensitive tinting | partial | P2 | 140–250 |
| [TUI-037](tui.md#tui-037---footer-git-metadata-asynchronous-refresh-and-extension-status-data) | Footer Git metadata, asynchronous refresh and extension status data | partial | P1 | 250–450 |
| [TUI-038](tui.md#tui-038---legacy-and-customized-footer-usage-cost-and-context-projection) | Legacy and customized footer usage, cost and context projection | partial | P1 | 250–450 |
| [TUI-039](tui.md#tui-039---compact-footer-priorities-runtime-badges-and-status-line-allocation) | Compact footer priorities, runtime badges and status-line allocation | missing | P2 | 200–350 |
| [TUI-040](tui.md#tui-040---responsive-session-header-and-live-startup-resource-summary) | Responsive session header and live startup resource summary | partial | P2 | 160–280 |
| [TUI-041](tui.md#tui-041---live-working-retry-and-compaction-indicators-with-countdown-disposal) | Live working, retry and compaction indicators with countdown disposal | partial | P1 | 300–500 |
| [TUI-042](tui.md#tui-042---bordered-extension-loader-cancellation-facade) | Bordered extension loader cancellation facade | partial | P2 | 60–120 |
| [TUI-043](tui.md#tui-043---completed-run-transcript-summary-and-verb-selection) | Completed-run transcript summary and verb selection | missing | P3 | 45–90 |
| [TUI-044](tui.md#tui-044---live-model-picker-favorites-scoped-view-and-selection-persistence) | Live model picker, favorites, scoped view and selection persistence | partial | P1 | 400–650 |
| [TUI-045](tui.md#tui-045---scoped-model-editor-unavailable-entries-and-session-only-versus-saved-order) | Scoped-model editor, unavailable entries and session-only versus saved order | partial | P1 | 380–620 |
| [TUI-046](tui.md#tui-046---authentication-option-picker-configured-status-labels-and-method-routing) | Authentication option picker, configured-status labels and method routing | partial | P2 | 220–360 |
| [TUI-047](tui.md#tui-047---provider-authentication-dialogs-prompt-callbacks-and-editor-restoration) | Provider authentication dialogs, prompt callbacks and editor restoration | partial | P1 | 320–520 |
| [TUI-048](tui.md#tui-048---application-keybinding-catalog-legacy-migration-and-configuration-loading) | Application keybinding catalog, legacy migration and configuration loading | partial | P1 | 260–430 |
| [TUI-049](tui.md#tui-049---custom-editor-action-precedence-shell-gutter-and-completed-mention-decoration) | Custom editor action precedence, shell gutter and completed-mention decoration | partial | P2 | 190–320 |
| [TUI-050](tui.md#tui-050---external-editor-process-lifecycle-terminal-handoff-and-draft-replacement) | External-editor process lifecycle, terminal handoff and draft replacement | partial | P1 | 130–230 |
| [TUI-051](tui.md#tui-051---responsive-shortcut-overview-and-effective-hotkey-transcript) | Responsive shortcut overview and effective hotkey transcript | partial | P3 | 100–170 |
| [TUI-052](tui.md#tui-052---notice-store-badge-counts-sticky-state-and-derived-group-dismissal) | Notice store, badge counts, sticky state and derived-group dismissal | missing | P1 | 160–250 |
| [TUI-053](tui.md#tui-053---diagnostic-reducers-console-capture-and-notices-attached-to-prompts) | Diagnostic reducers, console capture and notices attached to prompts | missing | P1 | 260–430 |
| [TUI-054](tui.md#tui-054---responsive-notices-inspector-filters-copy-and-dismissal) | Responsive notices inspector, filters, copy and dismissal | missing | P2 | 210–330 |
| [TUI-055](tui.md#tui-055---live-background-shell-list-details-foreground-follow-and-stop-controls) | Live background-shell list, details, foreground follow and stop controls | missing | P1 | 200–320 |
| [TUI-056](tui.md#tui-056---transcript-prompt-row-indexing-and-sticky-prompt-navigation) | Transcript prompt-row indexing and sticky prompt navigation | missing | P2 | 130–220 |
| [TUI-057](tui.md#tui-057---new-assistant-message-counts-and-jump-to-bottom-dock-pill) | New assistant-message counts and jump-to-bottom dock pill | partial | P2 | 90–160 |
| [TUI-058](tui.md#tui-058---context-occupancy-grid-proportional-bar-category-details-and-largest-entries) | Context occupancy grid, proportional bar, category details and largest entries | partial | P2 | 150–240 |
| [TUI-059](tui.md#tui-059---saved-versus-current-project-trust-selector-and-restart-semantics) | Saved versus current project trust selector and restart semantics | partial | P1 | 80–140 |
| [TUI-060](tui.md#tui-060---subagent-footer-projection-resident-counts-bounded-activity-and-attention-status) | Subagent footer projection, resident counts, bounded activity and attention status | missing | P1 | 150–250 |
| [TUI-061](tui.md#tui-061---user-message-fork-picker-and-asynchronous-fork-result-handoff) | User-message fork picker and asynchronous fork result handoff | partial | P1 | 100–170 |
| [TUI-062](tui.md#tui-062---session-fuzzy-quoted-phrase-and-regular-expression-search-with-ordering-modes) | Session fuzzy, quoted-phrase and regular-expression search with ordering modes | partial | P2 | 90–150 |
| [TUI-063](tui.md#tui-063---session-list-hierarchy-scopesortname-controls-and-path-aware-rendering) | Session list hierarchy, scope/sort/name controls and path-aware rendering | partial | P2 | 180–300 |
| [TUI-064](tui.md#tui-064---session-selector-loading-scope-caches-delete-confirmation-and-rename-lifetime) | Session selector loading, scope caches, delete confirmation and rename lifetime | partial | P1 | 180–290 |
| [TUI-065](tui.md#tui-065---session-tree-topology-filtered-ancestors-branch-folding-and-selection-preservation) | Session tree topology, filtered ancestors, branch folding and selection preservation | partial | P1 | 280–440 |
| [TUI-066](tui.md#tui-066---tree-row-viewport-entry-formatting-full-copy-and-inline-labels) | Tree row viewport, entry formatting, full copy and inline labels | partial | P2 | 220–350 |
| [TUI-067](tui.md#tui-067---tree-navigation-chooser-optional-summary-cancellation-and-editor-preservation) | Tree navigation chooser, optional summary cancellation and editor preservation | partial | P1 | 130–220 |
| [TUI-068](tui.md#tui-068---resource-config-picker-grouping-scope-switching-and-inherited-state-display) | Resource config picker grouping, scope switching and inherited-state display | missing | P2 | 210–340 |
| [TUI-069](tui.md#tui-069---resource-enable-filters-and-project-inheritloadunload-mutation-semantics) | Resource enable filters and project inherit/load/unload mutation semantics | missing | P1 | 180–290 |
| [TUI-070](tui.md#tui-070---first-run-theme-preview-and-explicit-analytics-choice-dialog) | First-run theme preview and explicit analytics choice dialog | partial | P2 | 90–150 |
| [TUI-071](tui.md#tui-071---embedded-portrait-easter-eggs-animation-timing-and-cleanup) | Embedded portrait Easter eggs, animation timing and cleanup | missing | P3 | 200–330 |
| [TUI-072](tui.md#tui-072---startup-changelog-baselines-collapsed-notices-and-explicit-history-display) | Startup changelog baselines, collapsed notices and explicit history display | partial | P2 | 70–140 |
| [TUI-073](tui.md#tui-073---extension-selector-input-and-multiline-editor-presentation-contracts) | Extension selector, input and multiline editor presentation contracts | partial | P1 | 190–310 |
| [TUI-074](tui.md#tui-074---prompt-admission-compaction-queues-follow-up-and-send-now-ownership) | Prompt admission, compaction queues, follow-up and send-now ownership | partial | P1 | 240–390 |
| [TUI-075](tui.md#tui-075---session-rebind-and-reload-presentation-ordering) | Session rebind and reload presentation ordering | partial | P1 | 180–290 |
| [TUI-076](tui.md#tui-076---signal-crash-suspend-and-normal-shutdown-sequencing) | Signal, crash, suspend and normal shutdown sequencing | partial | P1 | 210–340 |
| [TUI-077](tui.md#tui-077---bounded-tmux-keyboard-configuration-startup-diagnostics) | Bounded tmux keyboard-configuration startup diagnostics | missing | P2 | 45–85 |
| [TUI-078](tui.md#tui-078---session-sharing-host-gh-lifecycle-and-viewer-link-construction) | Session sharing host, gh lifecycle and viewer-link construction | partial | P2 | 170–280 |
| [TUI-079](tui.md#tui-079---live-main-screen-and-fullscreen-renderer-replacement) | Live main-screen and fullscreen renderer replacement | partial | P1 | 160–270 |
| [TUI-080](tui.md#tui-080---clipboard-imagetext-admission-and-latest-assistant-copy-feedback) | Clipboard image/text admission and latest-assistant copy feedback | partial | P1 | 100–180 |
| [TUI-081](tui.md#tui-081---session-import-export-clone-and-new-session-command-orchestration) | Session import, export, clone and new-session command orchestration | partial | P1 | 140–240 |
| [TUI-082](tui.md#tui-082---session-naming-and-expanded-identity-usage-and-cost-display) | Session naming and expanded identity, usage and cost display | partial | P2 | 80–150 |
| [TUI-083](tui.md#tui-083---fast-mode-command-capability-gating-and-completion-visibility) | Fast-mode command, capability gating and completion visibility | missing | P2 | 35–65 |
| [TUI-084](tui.md#tui-084---explicit-rendered-transcript-and-message-debug-snapshot-command) | Explicit rendered-transcript and message debug snapshot command | missing | P2 | 45–90 |
| [TUI-201](tui.md#tui-201---consistent-grapheme-cell-widths-and-output-only-normalization) | Consistent grapheme cell widths and output-only normalization | partial | P1 | 180–300 |
| [TUI-202](tui.md#tui-202---ansi-aware-word-wrapping-and-stable-style-carry-across-rows) | ANSI-aware word wrapping and stable style carry across rows | partial | P1 | 210–340 |
| [TUI-203](tui.md#tui-203---bounded-column-slicing-ellipsis-truncation-and-background-continuation) | Bounded column slicing, ellipsis truncation and background continuation | partial | P1 | 180–290 |
| [TUI-204](tui.md#tui-204---osc8-hyperlink-hit-testing-selective-removal-and-hover-styling) | OSC8 hyperlink hit-testing, selective removal and hover styling | partial | P1 | 70–120 |
| [TUI-205](tui.md#tui-205---padded-text-box-spacer-and-first-line-truncated-components) | Padded Text, Box, Spacer and first-line truncated components | partial | P1 | 160–260 |
| [TUI-206](tui.md#tui-206---legacy-kitty-and-modifyotherkeys-key-identity-and-printable-decoding) | Legacy, Kitty and modifyOtherKeys key identity and printable decoding | partial | P1 | 400–650 |
| [TUI-207](tui.md#tui-207---incremental-sequence-framing-and-bracketed-paste-assembly) | Incremental sequence framing and bracketed-paste assembly | partial | P1 | 280–430 |
| [TUI-208](tui.md#tui-208---keyboard-negotiation-platform-modifier-helpers-and-fallback-input) | Keyboard negotiation, platform modifier helpers and fallback input | partial | P1 | 240–400 |
| [TUI-209](tui.md#tui-209---terminal-shutdown-input-draining-progress-and-output-contract) | Terminal shutdown, input draining, progress and output contract | partial | P1 | 200–350 |
| [TUI-210](tui.md#tui-210---generic-keybinding-registry-defaults-and-conflict-semantics) | Generic keybinding registry, defaults and conflict semantics | partial | P1 | 120–220 |
| [TUI-211](tui.md#tui-211---stack-allocation-visibility-and-direct-rendering) | Stack allocation, visibility and direct rendering | partial | P1 | 220–360 |
| [TUI-212](tui.md#tui-212---extensible-constrained-layout-tree-and-nested-scroll-geometry) | Extensible constrained layout tree and nested scroll geometry | partial | P1 | 320–520 |
| [TUI-213](tui.md#tui-213---scroll-state-follow-end-and-render-notifications) | Scroll state, follow-end and render notifications | partial | P1 | 100–180 |
| [TUI-214](tui.md#tui-214---transient-scrollbars-proportional-geometry-and-cell-preserving-style) | Transient scrollbars, proportional geometry and cell-preserving style | partial | P1 | 180–280 |
| [TUI-215](tui.md#tui-215---ansi-safe-row-composition-blank-clearing-and-image-protection) | ANSI-safe row composition, blank clearing and image protection | partial | P1 | 80–150 |
| [TUI-216](tui.md#tui-216---renderer-lifecycle-frame-scheduling-and-restart-ownership) | Renderer lifecycle, frame scheduling and restart ownership | partial | P1 | 170–270 |
| [TUI-217](tui.md#tui-217---input-interception-focusable-delivery-and-hardware-cursor-markers) | Input interception, focusable delivery and hardware cursor markers | partial | P1 | 230–380 |
| [TUI-218](tui.md#tui-218---overlay-focus-stack-hidden-state-transitions-and-deferred-restoration) | Overlay focus stack, hidden-state transitions and deferred restoration | partial | P1 | 290–470 |
| [TUI-219](tui.md#tui-219---overlay-size-resolution-screen-anchoring-and-passive-placement) | Overlay size resolution, screen anchoring and passive placement | partial | P1 | 170–270 |
| [TUI-220](tui.md#tui-220---terminal-background-queries-and-color-scheme-reply-ownership) | Terminal background queries and color-scheme reply ownership | partial | P1 | 170–280 |
| [TUI-221](tui.md#tui-221---main-screen-differential-rendering-shrink-policy-and-transcript-cursor-accounting) | Main-screen differential rendering, shrink policy and transcript cursor accounting | partial | P1 | 300–500 |
| [TUI-222](tui.md#tui-222---alternate-screen-frame-assembly-teardown-transcript-and-mode-ownership) | Alternate-screen frame assembly, teardown transcript and mode ownership | partial | P1 | 220–360 |
| [TUI-223](tui.md#tui-223---viewport-wheel-routing-prompt-navigation-and-scrollbar-dragging) | Viewport wheel routing, prompt navigation and scrollbar dragging | partial | P1 | 230–370 |
| [TUI-224](tui.md#tui-224---application-text-selection-osc52-copy-and-internal-hyperlink-activation) | Application text selection, OSC52 copy and internal hyperlink activation | partial | P1 | 330–520 |
| [TUI-225](tui.md#tui-225---terminal-image-color-and-hyperlink-capability-detection) | Terminal image, color and hyperlink capability detection | partial | P1 | 130–220 |
| [TUI-226](tui.md#tui-226---kitty-and-iterm2-image-serialization-and-deletion) | Kitty and iTerm2 image serialization and deletion | partial | P0 | 140–240 |
| [TUI-227](tui.md#tui-227---kitty-metadata-generations-placement-reuse-and-vertical-cropping) | Kitty metadata generations, placement reuse and vertical cropping | missing | P1 | 200–330 |
| [TUI-228](tui.md#tui-228---image-dimensions-and-aspect-preserving-cell-fitting) | Image dimensions and aspect-preserving cell fitting | partial | P1 | 130–210 |
| [TUI-229](tui.md#tui-229---image-component-row-reservation-and-informative-fallback) | Image component, row reservation and informative fallback | partial | P1 | 150–250 |
| [TUI-230](tui.md#tui-230---loader-animation-configurable-frames-and-render-scheduling) | Loader animation, configurable frames and render scheduling | partial | P1 | 110–180 |
| [TUI-231](tui.md#tui-231---cancellable-loader-signal-and-configured-cancel-input) | Cancellable loader signal and configured cancel input | partial | P1 | 60–100 |
| [TUI-232](tui.md#tui-232---transient-alternate-screen-flash-messages) | Transient alternate-screen flash messages | missing | P2 | 65–110 |
| [TUI-233](tui.md#tui-233---unicode-word-navigation-and-atomic-segments) | Unicode word navigation and atomic segments | partial | P1 | 110–190 |
| [TUI-234](tui.md#tui-234---kill-ring-and-undo-snapshot-semantics) | Kill-ring and undo snapshot semantics | partial | P1 | 50–90 |
| [TUI-235](tui.md#tui-235---fuzzy-score-precision-token-matching-and-alphanumeric-fallback) | Fuzzy score precision, token matching and alphanumeric fallback | partial | P2 | 50–90 |
| [TUI-236](tui.md#tui-236---single-line-input-mutation-paste-and-undo-integration) | Single-line input mutation, paste and undo integration | partial | P1 | 200–330 |
| [TUI-237](tui.md#tui-237---single-line-input-horizontal-window-and-focus-cursor-rendering) | Single-line input horizontal window and focus cursor rendering | partial | P1 | 80–140 |
| [TUI-238](tui.md#tui-238---asynchronous-autocomplete-provider-and-slash-command-contracts) | Asynchronous autocomplete provider and slash-command contracts | partial | P1 | 160–270 |
| [TUI-239](tui.md#tui-239---direct-filesystem-completion-quoted-tokens-and-display-paths) | Direct filesystem completion, quoted tokens and display paths | partial | P1 | 180–290 |
| [TUI-240](tui.md#tui-240---recursive--attachment-search-ranking-and-cancellation) | Recursive @ attachment search, ranking and cancellation | missing | P1 | 180–300 |
| [TUI-241](tui.md#tui-241---autocomplete-application-and-quoted-directory-caret-placement) | Autocomplete application and quoted directory caret placement | partial | P1 | 80–140 |
| [TUI-242](tui.md#tui-242---editor-state-access-and-atomic-programmatic-replacement) | Editor state access and atomic programmatic replacement | partial | P1 | 100–170 |
| [TUI-243](tui.md#tui-243---editor-paste-framing-sanitization-and-large-paste-folding) | Editor paste framing, sanitization and large-paste folding | partial | P1 | 90–150 |
| [TUI-244](tui.md#tui-244---paste-marker-ownership-deletion-and-collision-free-registry-renumbering) | Paste-marker ownership, deletion and collision-free registry renumbering | partial | P0 | 140–230 |
| [TUI-245](tui.md#tui-245---editor-undo-transaction-boundaries-and-full-snapshot-restoration) | Editor undo transaction boundaries and full snapshot restoration | partial | P1 | 110–180 |
| [TUI-246](tui.md#tui-246---editor-history-browsing-with-preserved-draft-and-visual-boundary-routing) | Editor history browsing with preserved draft and visual-boundary routing | partial | P1 | 130–210 |
| [TUI-247](tui.md#tui-247---editor-input-precedence-newline-workarounds-and-submit-lifecycle) | Editor input precedence, newline workarounds and submit lifecycle | partial | P1 | 140–230 |
| [TUI-248](tui.md#tui-248---editor-grapheme-edits-and-unicode-safe-word-cursor-operations) | Editor grapheme edits and Unicode-safe word cursor operations | partial | P0 | 150–250 |
| [TUI-249](tui.md#tui-249---editor-kill-accumulation-and-multiline-yank-pop-replacement) | Editor kill accumulation and multiline yank-pop replacement | partial | P1 | 90–160 |
| [TUI-250](tui.md#tui-250---editor-marker-aware-word-wrapping-and-logical-to-visual-ranges) | Editor marker-aware word wrapping and logical-to-visual ranges | partial | P1 | 180–290 |
| [TUI-251](tui.md#tui-251---editor-render-only-decorations-and-concealed-source-prefixes) | Editor render-only decorations and concealed source prefixes | missing | P1 | 150–240 |
| [TUI-252](tui.md#tui-252---editor-prompt-gutter-borders-viewport-and-caret-rendering) | Editor prompt gutter, borders, viewport and caret rendering | partial | P1 | 170–280 |
| [TUI-253](tui.md#tui-253---editor-visual-row-movement-sticky-column-and-paste-continuation-traversal) | Editor visual-row movement, sticky column and paste continuation traversal | partial | P1 | 180–300 |
| [TUI-254](tui.md#tui-254---editor-character-jump-mode-and-utf-8-safe-search-boundaries) | Editor character-jump mode and UTF-8-safe search boundaries | partial | P0 | 80–140 |
| [TUI-255](tui.md#tui-255---editor-asynchronous-completion-scheduling-stale-result-checks-and-menu-acceptance) | Editor asynchronous completion scheduling, stale-result checks and menu acceptance | partial | P1 | 250–410 |
| [TUI-256](tui.md#tui-256---selectlist-filtering-configured-navigation-and-callback-lifetime) | SelectList filtering, configured navigation and callback lifetime | partial | P1 | 80–140 |
| [TUI-257](tui.md#tui-257---selectlist-adaptive-columns-custom-truncation-and-shared-chrome) | SelectList adaptive columns, custom truncation and shared chrome | partial | P1 | 90–150 |
| [TUI-258](tui.md#tui-258---settingslist-search-live-item-patching-value-cycling-and-submenu-ownership) | SettingsList search, live item patching, value cycling and submenu ownership | partial | P1 | 140–230 |
| [TUI-259](tui.md#tui-259---settingslist-responsive-layout-selected-descriptions-and-hint-chrome) | SettingsList responsive layout, selected descriptions and hint chrome | partial | P1 | 110–180 |
| [TUI-260](tui.md#tui-260---markdown-transform-cache-padding-and-default-style-lifecycle) | Markdown transform, cache, padding and default-style lifecycle | partial | P1 | 140–230 |
| [TUI-261](tui.md#tui-261---markdown-inline-syntax-inherited-styles-and-capability-aware-hyperlinks) | Markdown inline syntax, inherited styles and capability-aware hyperlinks | partial | P1 | 160–260 |
| [TUI-262](tui.md#tui-262---markdown-nested-blocks-list-markers-quote-continuation-and-spacing) | Markdown nested blocks, list markers, quote continuation and spacing | partial | P1 | 240–390 |
| [TUI-263](tui.md#tui-263---markdown-fenced-code-streaming-highlighting-and-native-mermaid-preservation) | Markdown fenced code streaming, highlighting and native Mermaid preservation | partial | P1 | 160–270 |
| [TUI-264](tui.md#tui-264---markdown-table-column-budgets-and-lossless-multiline-cells) | Markdown table column budgets and lossless multiline cells | partial | P1 | 200–320 |
| [TUI-265](tui.md#tui-265---markdown-table-style-policy-and-default-invalidation) | Markdown table style policy and default invalidation | partial | P1 | 90–160 |
| [TUI-266](tui.md#tui-266---bounded-kitty-upload-reuse-and-placement-redraw-transactions) | Bounded Kitty upload reuse and placement redraw transactions | missing | P1 | 150–250 |
| [TUI-267](tui.md#tui-267---shared-chrome-border-families-and-narrow-width-gate) | Shared chrome border families and narrow-width gate | missing | P2 | 75–130 |

## SUB

| ID | Contract | Status | Priority | Estimated Rust lines |
|---|---|---|---|---:|
| [SUB-001](subagents.md#sub-001---strict-native-markdown-and-json-agent-definitions-with-source-identity) | Strict native Markdown and JSON agent definitions with source identity | missing | P1 | 240–390 |
| [SUB-002](subagents.md#sub-002---opt-in-claude-agent-compatibility-with-explicit-model-aliases) | Opt-in Claude agent compatibility with explicit model aliases | missing | P2 | 100–180 |
| [SUB-003](subagents.md#sub-003---bounded-agent-discovery-trust-filtering-and-whole-definition-precedence) | Bounded agent discovery, trust filtering and whole-definition precedence | missing | P1 | 330–520 |
| [SUB-004](subagents.md#sub-004---built-in-agents-and-incremental-model-visible-catalog) | Built-in agents and incremental model-visible catalog | missing | P1 | 140–230 |
| [SUB-005](subagents.md#sub-005---strict-scoped-v2-settings-and-monotonic-numeric-limits) | Strict scoped V2 settings and monotonic numeric limits | missing | P1 | 150–240 |
| [SUB-006](subagents.md#sub-006---permission-rule-grammar-tool-subjects-and-conservative-shell-decomposition) | Permission rule grammar, tool subjects and conservative shell decomposition | missing | P0 | 230–370 |
| [SUB-007](subagents.md#sub-007---child-permission-ceilings-and-source-sensitive-authority-raisers) | Child permission ceilings and source-sensitive authority raisers | partial | P0 | 140–230 |
| [SUB-008](subagents.md#sub-008---approval-broker-with-foreground-prompts-inbox-routing-and-owner-cancellation) | Approval broker with foreground prompts, inbox routing and owner cancellation | missing | P0 | 180–290 |
| [SUB-009](subagents.md#sub-009---startup-authorization-for-sub-yolo-and-reduced-delegated-shell-environment) | Startup authorization for sub-yolo and reduced delegated shell environment | missing | P0 | 130–220 |
| [SUB-010](subagents.md#sub-010---strict-hook-configuration-source-precedence-and-revocable-handler-registration) | Strict hook configuration, source precedence and revocable handler registration | missing | P1 | 180–300 |
| [SUB-011](subagents.md#sub-011---bounded-hook-dispatch-durable-intent-and-fail-closed-decision-events) | Bounded hook dispatch, durable intent and fail-closed decision events | missing | P0 | 260–430 |
| [SUB-012](subagents.md#sub-012---host-pinned-command-hooks-with-literal-argv-and-bounded-subprocess-io) | Host-pinned command hooks with literal argv and bounded subprocess I/O | missing | P0 | 220–360 |
| [SUB-013](subagents.md#sub-013---frozen-child-runtime-contract-and-hash-checked-transcript-initialization) | Frozen child runtime contract and hash-checked transcript initialization | missing | P0 | 230–380 |
| [SUB-014](subagents.md#sub-014---explicit-child-safe-adapters-mcp-bridge-and-capability-fingerprints) | Explicit child-safe adapters, MCP bridge and capability fingerprints | partial | P0 | 300–480 |
| [SUB-015](subagents.md#sub-015---scoped-file-tools-and-guarded-v2-child-session-construction) | Scoped file tools and guarded V2 child session construction | partial | P0 | 250–410 |
| [SUB-016](subagents.md#sub-016---owner-scoped-child-registry-generation-checks-parking-and-tombstones) | Owner-scoped child registry, generation checks, parking and tombstones | partial | P0 | 280–450 |
| [SUB-017](subagents.md#sub-017---durable-child-completions-parent-delivery-confirmation-and-interruption-notices) | Durable child completions, parent delivery confirmation and interruption notices | missing | P1 | 240–400 |
| [SUB-018](subagents.md#sub-018---launch-preflight-frozen-model-choice-unique-names-and-staged-batch-acceptance) | Launch preflight, frozen model choice, unique names and staged batch acceptance | partial | P0 | 360–590 |
| [SUB-019](subagents.md#sub-019---live-authority-checks-external-paths-and-approval-revalidation-at-dispatch) | Live authority checks, external paths and approval revalidation at dispatch | partial | P0 | 270–440 |
| [SUB-020](subagents.md#sub-020---admission-liveness-deadlines-and-abandonment-of-uncooperative-child-turns) | Admission, liveness deadlines and abandonment of uncooperative child turns | partial | P0 | 340–540 |
| [SUB-021](subagents.md#sub-021---model-request-budgets-one-tool-free-wrap-up-and-temporary-retry-suppression) | Model-request budgets, one tool-free wrap-up and temporary retry suppression | partial | P1 | 170–280 |
| [SUB-022](subagents.md#sub-022---resume-steering-stop-and-deletion-of-persistent-child-instances) | Resume, steering, stop and deletion of persistent child instances | partial | P1 | 230–390 |
| [SUB-023](subagents.md#sub-023---agent-tool-batches-background-acknowledgement-and-foreground-detachment) | Agent tool batches, background acknowledgement and foreground detachment | missing | P1 | 240–390 |
| [SUB-024](subagents.md#sub-024---agent-control-queries-range-output-and-repeated-wait-suppression) | Agent control queries, range output and repeated-wait suppression | missing | P1 | 180–290 |
| [SUB-025](subagents.md#sub-025---selected-child-resources-and-explicit-untrusted-context-packets) | Selected child resources and explicit untrusted context packets | missing | P1 | 160–280 |
| [SUB-026](subagents.md#sub-026---bounded-report-envelope-and-local-output-schema-validation) | Bounded report envelope and local output-schema validation | missing | P1 | 210–350 |
| [SUB-027](subagents.md#sub-027---per-turn-observed-file-evidence-acceptance-criteria-and-honest-assurance) | Per-turn observed file evidence, acceptance criteria and honest assurance | missing | P1 | 210–340 |
| [SUB-028](subagents.md#sub-028---owner-scoped-output-artifacts-utf-8-range-reads-and-durable-retention) | Owner-scoped output artifacts, UTF-8 range reads and durable retention | missing | P1 | 270–450 |
| [SUB-029](subagents.md#sub-029---deterministic-assurance-helpers-and-bounded-batch-synthesis) | Deterministic assurance helpers and bounded batch synthesis | missing | P1 | 320–500 |
| [SUB-030](subagents.md#sub-030---raw-dirty-tree-snapshots-and-owned-isolated-git-worktrees) | Raw dirty-tree snapshots and owned isolated Git worktrees | missing | P0 | 430–680 |
| [SUB-031](subagents.md#sub-031---pinned-immutable-patch-artifacts-and-private-recovery-records) | Pinned immutable patch artifacts and private recovery records | missing | P0 | 190–310 |
| [SUB-032](subagents.md#sub-032---authorized-patch-integration-verification-and-conflict-preserving-rollback) | Authorized patch integration, verification and conflict-preserving rollback | missing | P0 | 420–700 |
| [SUB-033](subagents.md#sub-033---agent-catalog-cli-definition-creation-and-conservative-orphan-pruning) | Agent catalog CLI, definition creation and conservative orphan pruning | missing | P1 | 250–400 |
| [SUB-034](subagents.md#sub-034---sdk-composition-and-versioned-trusted-extension-controller) | SDK composition and versioned trusted-extension controller | partial | P1 | 200–340 |
| [SUB-035](subagents.md#sub-035---subagent-extension-binding-session-catalog-refresh-and-lifecycle-events) | Subagent extension binding, session catalog refresh and lifecycle events | missing | P1 | 300–480 |
| [SUB-036](subagents.md#sub-036---bounded-child-view-bridge-historical-transcripts-and-live-input-control) | Bounded child view bridge, historical transcripts and live input control | missing | P1 | 370–590 |
| [SUB-037](subagents.md#sub-037---agent-hub-roster-child-inspector-and-scoped-approval-controls) | Agent Hub roster, child inspector and scoped approval controls | missing | P2 | 240–380 |
| [SUB-038](subagents.md#sub-038---bounded-ruflo-agent-pack-conversion-and-generated-format-incompatibility) | Bounded Ruflo agent-pack conversion and generated-format incompatibility | missing | P2 | 150–240 |
| [SUB-039](subagents.md#sub-039---frozen-task-contracts-and-explicit-qualityreviewer-recommendations) | Frozen task contracts and explicit quality/reviewer recommendations | missing | P2 | 150–240 |
| [SUB-040](subagents.md#sub-040---bounded-instruction-chain-manifest-and-repository-orientation-helper) | Bounded instruction-chain manifest and repository orientation helper | missing | P2 | 230–380 |
| [SUB-041](subagents.md#sub-041---legacy-child-model-routing-captured-capability-identity-and-sampling-helpers) | Legacy child model routing, captured capability identity and sampling helpers | missing | P2 | 140–230 |
| [SUB-042](subagents.md#sub-042---legacy-fork-prompt-identity-report-validation-and-schema-v1-report-artifacts) | Legacy fork prompt identity, report validation and schema-v1 report artifacts | missing | P2 | 250–420 |
| [SUB-043](subagents.md#sub-043---declarative-environmentpathcommand-preflight-helper) | Declarative environment/path/command preflight helper | missing | P2 | 100–170 |
| [SUB-044](subagents.md#sub-044---shared-child-activity-hook-flushing-lifecycle-races-and-legacy-error-envelopes) | Shared child activity, hook flushing, lifecycle races and legacy error envelopes | partial | P1 | 160–270 |

## SAFE

| ID | Contract | Status | Priority | Estimated Rust lines |
|---|---|---|---|---:|
| [SAFE-001](safe-verify.md#safe-001---mandatory-guard-bootstrap-and-process-scoped-flags) | Mandatory guard bootstrap and process-scoped flags | missing | P0 | 120–220 |
| [SAFE-002](safe-verify.md#safe-002---canonical-guarded-root-and-immutable-git-baseline) | Canonical guarded root and immutable Git baseline | missing | P0 | 80–160 |
| [SAFE-003](safe-verify.md#safe-003---version-3-durable-guard-state-and-branch-restoration) | Version 3 durable guard state and branch restoration | missing | P0 | 240–420 |
| [SAFE-004](safe-verify.md#safe-004---effective-tool-authority-by-mode-origin-and-declared-access) | Effective tool authority by mode, origin and declared access | missing | P0 | 180–300 |
| [SAFE-005](safe-verify.md#safe-005---prospective-path-and-protected-file-mutation-checks) | Prospective-path and protected-file mutation checks | missing | P0 | 160–280 |
| [SAFE-006](safe-verify.md#safe-006---manual-planbuild-transitions-and-pending-plan-override) | Manual plan/build transitions and pending-plan override | missing | P1 | 100–180 |
| [SAFE-007](safe-verify.md#safe-007---planningexecution-model-selection-and-thinking-restoration) | Planning/execution model selection and thinking restoration | missing | P1 | 100–200 |
| [SAFE-008](safe-verify.md#safe-008---hidden-planning-and-approved-plan-context-messages) | Hidden planning and approved-plan context messages | missing | P1 | 70–130 |
| [SAFE-009](safe-verify.md#safe-009---ask-planning-questionnaire) | ask planning questionnaire | missing | P1 | 130–230 |
| [SAFE-010](safe-verify.md#safe-010---draft_plan-bounded-session-native-markdown-drafts) | draft_plan bounded session-native Markdown drafts | missing | P1 | 100–180 |
| [SAFE-011](safe-verify.md#safe-011---propose_plan-approval-submission-and-retry-behavior) | propose_plan approval submission and retry behavior | missing | P1 | 100–180 |
| [SAFE-012](safe-verify.md#safe-012---explicit-review-approval-identity-and-settled-turn-handoff) | Explicit review, approval identity and settled-turn handoff | missing | P0 | 180–300 |
| [SAFE-013](safe-verify.md#safe-013---read_plan-acknowledgement-and-post-compaction-mutation-gate) | read_plan acknowledgement and post-compaction mutation gate | missing | P0 | 80–150 |
| [SAFE-014](safe-verify.md#safe-014---fresh-approved-plan-context-projection-and-orphan-result-pruning) | Fresh approved-plan context projection and orphan-result pruning | missing | P1 | 80–150 |
| [SAFE-015](safe-verify.md#safe-015---compacted-and-retained-approval-context-handoffs) | Compacted and retained approval-context handoffs | missing | P1 | 70–130 |
| [SAFE-016](safe-verify.md#safe-016---bounded-prose-only-planning-continuations-and-abort-suppression) | Bounded prose-only planning continuations and abort suppression | missing | P1 | 90–160 |
| [SAFE-017](safe-verify.md#safe-017---authorized-native-mutation-generations) | Authorized native mutation generations | missing | P0 | 90–170 |
| [SAFE-018](safe-verify.md#safe-018---subagent-patch-verification-import-and-conflict-accounting) | Subagent patch verification import and conflict accounting | missing | P0 | 90–180 |
| [SAFE-019](safe-verify.md#safe-019---trusted-verifier-argv-validation-and-command-identity) | Trusted verifier argv validation and command identity | missing | P0 | 70–130 |
| [SAFE-020](safe-verify.md#safe-020---bounded-verifier-subprocess-timeout-and-cancellation) | Bounded verifier subprocess, timeout and cancellation | missing | P0 | 220–380 |
| [SAFE-021](safe-verify.md#safe-021---automaticmanual-verification-durable-receipts-and-lifecycle-cleanup) | Automatic/manual verification, durable receipts and lifecycle cleanup | missing | P0 | 190–330 |
| [SAFE-022](safe-verify.md#safe-022---versioned-goals-host-registry-and-lifecycle-ownership) | Versioned Goals host registry and lifecycle ownership | missing | P1 | 70–130 |
| [SAFE-023](safe-verify.md#safe-023---immutable-goals-authority-snapshot-and-session-binding) | Immutable Goals authority snapshot and session binding | missing | P0 | 70–140 |
| [SAFE-024](safe-verify.md#safe-024---goals-fresh-verification-with-overlap-cancellation-and-stale-pass-rejection) | Goals fresh verification with overlap, cancellation and stale-pass rejection | missing | P0 | 140–250 |
| [SAFE-025](safe-verify.md#safe-025---authoritative-frozen-capability-publication-to-subagents) | Authoritative frozen capability publication to subagents | missing | P0 | 50–100 |
| [SAFE-026](safe-verify.md#safe-026---guard-status-reporting-and-safety-notifications) | Guard status reporting and safety notifications | missing | P2 | 50–100 |

## OBS

| ID | Contract | Status | Priority | Estimated Rust lines |
|---|---|---|---|---:|
| [OBS-001](observability.md#obs-001---install-telemetry-gate-and-best-effort-version-ping) | Install telemetry gate and best-effort version ping | missing | P3 | 80–180 |
| [OBS-002](observability.md#obs-002---startup-timing-namespaces-and-emission-timing) | Startup timing namespaces and emission timing | partial | P2 | 20–70 |
| [OBS-003](observability.md#obs-003---always-on-durable-recording-and-opt-in-dashboard-startup) | Always-on durable recording and opt-in dashboard startup | missing | P1 | 140–280 |
| [OBS-004](observability.md#obs-004---recorder-identity-event-contracts-and-bounded-live-state) | Recorder identity, event contracts and bounded live state | missing | P1 | 280–480 |
| [OBS-005](observability.md#obs-005---user-skill-and-assistant-message-projection-with-usage-accumulation) | User, skill and assistant-message projection with usage accumulation | missing | P1 | 130–240 |
| [OBS-006](observability.md#obs-006---one-row-tool-lifecycle-transient-progress-and-forensic-result-storage) | One-row tool lifecycle, transient progress and forensic result storage | missing | P1 | 170–300 |
| [OBS-007](observability.md#obs-007---compaction-retry-and-queue-activity-projection) | Compaction, retry and queue activity projection | missing | P2 | 110–200 |
| [OBS-008](observability.md#obs-008---cognee-observation-correlation-and-recall-evidence) | Cognee observation correlation and recall evidence | missing | P2 | 110–190 |
| [OBS-009](observability.md#obs-009---goal-and-verifier-transition-projection-with-exact-deduplication) | Goal and verifier transition projection with exact deduplication | missing | P1 | 160–280 |
| [OBS-010](observability.md#obs-010---subagent-threads-completion-rows-and-extension-custom-messages) | Subagent threads, completion rows and extension custom messages | missing | P1 | 140–250 |
| [OBS-011](observability.md#obs-011---sqlite-runevent-schema-and-synchronous-append-semantics) | SQLite run/event schema and synchronous append semantics | missing | P1 | 220–360 |
| [OBS-012](observability.md#obs-012---durable-redaction-retention-pruning-and-sqlite-open-policy) | Durable redaction, retention pruning and SQLite open policy | missing | P1 | 130–240 |
| [OBS-013](observability.md#obs-013---historical-run-queries-and-ordered-replay-reconstruction) | Historical run queries and ordered replay reconstruction | missing | P1 | 180–320 |
| [OBS-014](observability.md#obs-014---loopback-dashboard-authentication-token-file-and-html-asset-loading) | Loopback dashboard authentication, token file and HTML asset loading | missing | P1 | 180–320 |
| [OBS-015](observability.md#obs-015---read-only-state-run-replay-and-server-sent-event-routes) | Read-only state, run replay and Server-Sent Event routes | missing | P1 | 230–380 |
| [OBS-016](observability.md#obs-016---combined-live-runs-cross-process-polling-and-shared-port-lifecycle) | Combined live runs, cross-process polling and shared-port lifecycle | missing | P1 | 200–340 |
| [OBS-017](observability.md#obs-017---lazy-shared-cognee-connection-and-dashboard-source-recovery) | Lazy shared Cognee connection and dashboard-source recovery | missing | P1 | 150–250 |
| [OBS-018](observability.md#obs-018---cognee-dataset-summary-cache-bounded-previews-and-graph-coalescing) | Cognee dataset summary cache, bounded previews and graph coalescing | missing | P2 | 160–280 |
| [OBS-019](observability.md#obs-019---cognee-http-routes-exact-validation-and-cancellationerror-boundary) | Cognee HTTP routes, exact validation and cancellation/error boundary | missing | P1 | 180–320 |
| [OBS-020](observability.md#obs-020---bounded-activity-tape-exact-search-and-stable-incremental-rendering) | Bounded activity tape, exact search and stable incremental rendering | missing | P2 | 450–750 |
| [OBS-021](observability.md#obs-021---forensic-inspector-tool-diff-rendering-and-responsive-pane-controls) | Forensic inspector, tool diff rendering and responsive pane controls | missing | P2 | 400–700 |
| [OBS-022](observability.md#obs-022---epoch-aligned-activity-pulse-per-agent-traces-and-header-metrics) | Epoch-aligned activity pulse, per-agent traces and header metrics | missing | P2 | 220–380 |
| [OBS-023](observability.md#obs-023---dashboard-run-switching-live-event-filtering-and-follow-controls) | Dashboard run switching, live-event filtering and follow controls | missing | P2 | 180–320 |
| [OBS-024](observability.md#obs-024---knowledge-view-lifecycle-coalesced-polling-and-memory-triggered-refresh) | Knowledge view lifecycle, coalesced polling and memory-triggered refresh | missing | P2 | 230–400 |
| [OBS-025](observability.md#obs-025---knowledge-document-previews-session-detail-and-context-query-inspector) | Knowledge document previews, session detail and context-query inspector | missing | P2 | 300–500 |
| [OBS-026](observability.md#obs-026---dataset-summary-table-and-explicit-cache-refresh-semantics) | Dataset summary table and explicit cache-refresh semantics | missing | P3 | 70–130 |
| [OBS-027](observability.md#obs-027---knowledge-graph-loading-type-filtering-and-incremental-dataset-updates) | Knowledge graph loading, type filtering and incremental dataset updates | missing | P2 | 240–420 |
| [OBS-028](observability.md#obs-028---bounded-frame-barneshut-graph-layout-and-level-of-detail-drawing) | Bounded-frame Barnes–Hut graph layout and level-of-detail drawing | missing | P3 | 250–450 |
| [OBS-029](observability.md#obs-029---graph-navigation-directional-relationships-and-document-to-entity-tracing) | Graph navigation, directional relationships and document-to-entity tracing | missing | P2 | 260–440 |

## COG

| ID | Contract | Status | Priority | Estimated Rust lines |
|---|---|---|---|---:|
| [COG-001](cognee.md#cog-001---cognee-configuration-defaults-validation-and-precedence) | Cognee configuration defaults, validation and precedence | missing | P1 | 150–250 |
| [COG-002](cognee.md#cog-002---shared-cognee-environment-file-parser-and-local-url-alias) | Shared Cognee environment-file parser and local URL alias | missing | P1 | 60–110 |
| [COG-003](cognee.md#cog-003---credential-resolution-same-origin-caches-and-connection-fallback) | Credential resolution, same-origin caches and connection fallback | missing | P0 | 110–180 |
| [COG-004](cognee.md#cog-004---project-dataset-naming-and-lexical-root-identity) | Project dataset naming and lexical-root identity | missing | P1 | 50–90 |
| [COG-005](cognee.md#cog-005---host-bound-cognee-session-ids-and-collision-resistant-sanitization) | Host-bound Cognee session IDs and collision-resistant sanitization | missing | P0 | 40–80 |
| [COG-006](cognee.md#cog-006---cognee-config-file-loading-and-private-atomic-persistence) | Cognee config file loading and private atomic persistence | missing | P1 | 60–110 |
| [COG-007](cognee.md#cog-007---credential-redaction-memory-envelopes-and-utf-16-caps) | Credential redaction, memory envelopes and UTF-16 caps | missing | P0 | 60–110 |
| [COG-008](cognee.md#cog-008---capture-serialization-and-visible-message-text-extraction) | Capture serialization and visible-message text extraction | missing | P1 | 30–65 |
| [COG-009](cognee.md#cog-009---cognee-http-url-authentication-and-error-taxonomy) | Cognee HTTP URL, authentication and error taxonomy | missing | P1 | 65–120 |
| [COG-010](cognee.md#cog-010---single-deadline-http-cancellation-and-late-operation-cleanup) | Single-deadline HTTP cancellation and late-operation cleanup | missing | P0 | 130–230 |
| [COG-011](cognee.md#cog-011---streaming-byte-limits-content-length-checks-and-truncating-utf-8-reads) | Streaming byte limits, Content-Length checks and truncating UTF-8 reads | missing | P0 | 120–210 |
| [COG-012](cognee.md#cog-012---recall-envelope-normalization-and-graph-context-condensation) | Recall envelope normalization and graph-context condensation | missing | P1 | 75–130 |
| [COG-013](cognee.md#cog-013---never-cognified-dataset-404-as-empty-memory) | Never-cognified dataset 404 as empty memory | missing | P1 | 25–50 |
| [COG-014](cognee.md#cog-014---datasetsession-scoped-recall-request-and-aggregate-output-cap) | Dataset/session-scoped recall request and aggregate output cap | missing | P1 | 70–120 |
| [COG-015](cognee.md#cog-015---permanent-memory-multipart-remember-request) | Permanent-memory multipart remember request | missing | P1 | 45–85 |
| [COG-016](cognee.md#cog-016---session-cache-entry-write-and-tolerant-acknowledgement) | Session-cache entry write and tolerant acknowledgement | missing | P1 | 60–110 |
| [COG-017](cognee.md#cog-017---background-improve-request-and-acceptance-semantics) | Background improve request and acceptance semantics | missing | P1 | 40–75 |
| [COG-018](cognee.md#cog-018---optional-server-agent-registration) | Optional server agent registration | missing | P1 | 35–65 |
| [COG-019](cognee.md#cog-019---optional-server-agent-unregistration) | Optional server agent unregistration | missing | P1 | 25–50 |
| [COG-020](cognee.md#cog-020---server-model-identity-projection-without-credential-exposure) | Server model identity projection without credential exposure | missing | P1 | 30–60 |
| [COG-021](cognee.md#cog-021---browse-api-data-contracts-id-validation-and-common-read-policy) | Browse API data contracts, ID validation and common read policy | missing | P1 | 120–210 |
| [COG-022](cognee.md#cog-022---dataset-enumeration-projection) | Dataset enumeration projection | missing | P1 | 25–45 |
| [COG-023](cognee.md#cog-023---dataset-document-enumeration-without-server-filesystem-paths) | Dataset-document enumeration without server filesystem paths | missing | P1 | 30–55 |
| [COG-024](cognee.md#cog-024---multi-dataset-status-request-and-empty-input-fast-path) | Multi-dataset status request and empty-input fast path | missing | P1 | 25–50 |
| [COG-025](cognee.md#cog-025---bounded-raw-document-preview) | Bounded raw document preview | missing | P1 | 30–60 |
| [COG-026](cognee.md#cog-026---context-only-dashboard-search) | Context-only dashboard search | missing | P1 | 45–85 |
| [COG-027](cognee.md#cog-027---graph-projection-degree-based-node-cap-and-dangling-edge-removal) | Graph projection, degree-based node cap and dangling-edge removal | missing | P1 | 90–160 |
| [COG-028](cognee.md#cog-028---dataset-graph-fetch-with-large-bounded-transport) | Dataset graph fetch with large bounded transport | missing | P1 | 25–50 |
| [COG-029](cognee.md#cog-029---session-cache-enumeration) | Session-cache enumeration | missing | P1 | 25–50 |
| [COG-030](cognee.md#cog-030---session-cache-detail-qa-and-trace-projection) | Session-cache detail, QA and trace projection | missing | P1 | 45–85 |
| [COG-031](cognee.md#cog-031---independent-recall-and-remember-circuit-breakers) | Independent recall and remember circuit breakers | missing | P1 | 45–85 |
| [COG-032](cognee.md#cog-032---durable-remember-operation-identity-and-record-schema) | Durable remember operation identity and record schema | missing | P1 | 35–65 |
| [COG-033](cognee.md#cog-033---pending-remember-queue-admission-and-capacity) | Pending remember queue admission and capacity | missing | P1 | 40–70 |
| [COG-034](cognee.md#cog-034---pending-queue-loading-uncertain-state-update-and-removal) | Pending queue loading, uncertain-state update and removal | missing | P0 | 90–150 |
| [COG-035](cognee.md#cog-035---single-flight-pending-drain-and-uncertain-write-quarantine) | Single-flight pending drain and uncertain write quarantine | missing | P0 | 160–260 |
| [COG-036](cognee.md#cog-036---failed-session-cache-warmup-buffer-and-bounded-replay) | Failed session-cache warmup buffer and bounded replay | missing | P0 | 100–180 |
| [COG-037](cognee.md#cog-037---dream-ledger-records-revision-validation-and-dataset-qualified-filenames) | Dream ledger records, revision validation and dataset-qualified filenames | missing | P1 | 140–220 |
| [COG-038](cognee.md#cog-038---cross-process-dream-capture-locking-and-monotonic-revisions) | Cross-process dream capture locking and monotonic revisions | missing | P0 | 120–200 |
| [COG-039](cognee.md#cog-039---revision-fenced-dream-claims-and-expiring-improve-leases) | Revision-fenced dream claims and expiring improve leases | missing | P0 | 95–165 |
| [COG-040](cognee.md#cog-040---dream-completion-and-compare-token-lease-release) | Dream completion and compare-token lease release | missing | P0 | 115–195 |
| [COG-041](cognee.md#cog-041---dream-candidate-scan-expiry-cleanup-and-dataset-sensitive-exclusion) | Dream candidate scan, expiry cleanup and dataset-sensitive exclusion | missing | P1 | 100–175 |
| [COG-042](cognee.md#cog-042---private-atomic-single-file-writer) | Private atomic single-file writer | missing | P1 | 40–80 |
| [COG-043](cognee.md#cog-043---extension-runtime-initialization-single-flight-loading-and-tracked-work) | Extension runtime initialization, single-flight loading and tracked work | missing | P1 | 180–290 |
| [COG-044](cognee.md#cog-044---asynchronous-cache-capture-with-success-counters-and-dream-revisions) | Asynchronous cache capture with success counters and dream revisions | missing | P0 | 90–160 |
| [COG-045](cognee.md#cog-045---health-probe-and-doctor-report-projection) | Health probe and doctor report projection | missing | P0 | 100–175 |
| [COG-046](cognee.md#cog-046---per-prompt-recall-exact-deduplication-and-transient-system-injection) | Per-prompt recall, exact deduplication and transient system injection | missing | P0 | 190–310 |
| [COG-047](cognee.md#cog-047---allowlisted-per-turn-tool-trace-capture) | Allowlisted per-turn tool trace capture | missing | P1 | 95–160 |
| [COG-048](cognee.md#cog-048---prompt-and-last-assistant-answer-pairing) | Prompt and last-assistant-answer pairing | missing | P1 | 65–115 |
| [COG-049](cognee.md#cog-049---current-session-idle-improvement-and-cooldown) | Current-session idle improvement and cooldown | missing | P1 | 140–230 |
| [COG-050](cognee.md#cog-050---old-session-dream-pass-with-bounded-claims-and-stop-on-error) | Old-session dream pass with bounded claims and stop-on-error | missing | P1 | 120–200 |
| [COG-051](cognee.md#cog-051---session-scoped-checkpoint-recall-handoff-for-blackhole) | Session-scoped checkpoint recall handoff for Blackhole | missing | P0 | 190–300 |
| [COG-052](cognee.md#cog-052---compaction-ownership-and-deterministic-local-anchor) | Compaction ownership and deterministic local anchor | missing | P1 | 130–220 |
| [COG-053](cognee.md#cog-053---post-compaction-checkpoint-injection-capture-and-permanent-queueing) | Post-compaction checkpoint injection, capture and permanent queueing | missing | P1 | 115–195 |
| [COG-054](cognee.md#cog-054---shutdown-drain-final-improvement-and-unregister-ordering) | Shutdown drain, final improvement and unregister ordering | missing | P0 | 70–130 |
| [COG-055](cognee.md#cog-055---model-facing-cognee_search-tool) | Model-facing cognee_search tool | missing | P1 | 80–145 |
| [COG-056](cognee.md#cog-056---cognee-slash-command-grammar-and-branch-precedence) | Cognee slash-command grammar and branch precedence | missing | P1 | 65–115 |
| [COG-057](cognee.md#cog-057---namespaced-cognee-settings-section-and-asynchronous-changes) | Namespaced Cognee settings section and asynchronous changes | missing | P1 | 95–160 |
| [COG-058](cognee.md#cog-058---bundled-cognee-skills-and-operational-documentation) | Bundled Cognee skills and operational documentation | missing | P2 | 35–70 |
| [COG-059](cognee.md#cog-059---cognee-observations-redaction-boundary-and-serialized-rotating-append) | Cognee observations, redaction boundary and serialized rotating append | missing | P0 | 160–260 |
| [COG-060](cognee.md#cog-060---observation-replay-and-live-subscription-semantics) | Observation replay and live subscription semantics | missing | P1 | 75–130 |
| [COG-061](cognee.md#cog-061---cognee-status-line-and-stale-context-safe-activity-spinner) | Cognee status line and stale-context-safe activity spinner | missing | P2 | 75–135 |
| [COG-062](cognee.md#cog-062---nonblocking-session-startup-and-optional-server-registration) | Nonblocking session startup and optional server registration | missing | P1 | 120–200 |
| [COG-063](cognee.md#cog-063---manual-cognee-search-command) | Manual Cognee search command | missing | P1 | 45–85 |
| [COG-064](cognee.md#cog-064---manual-remember-and-failure-queueing-policy) | Manual remember and failure queueing policy | missing | P0 | 70–125 |
| [COG-065](cognee.md#cog-065---manual-improve-bypass-of-automatic-scheduling-gates) | Manual improve bypass of automatic scheduling gates | missing | P0 | 45–85 |
| [COG-066](cognee.md#cog-066---policy-toggle-and-compaction-mode-slash-commands) | Policy toggle and compaction-mode slash commands | missing | P1 | 60–105 |
| [COG-067](cognee.md#cog-067---status-doctor-and-dashboard-handoff-commands) | Status, doctor and dashboard handoff commands | missing | P2 | 55–100 |
| [COG-068](cognee.md#cog-068---explicit-pending-or-uncertain-queue-flush-command) | Explicit pending or uncertain queue flush command | missing | P0 | 25–50 |

## BLACK

| ID | Contract | Status | Priority | Estimated Rust lines |
|---|---|---|---|---:|
| [BLACK-001](blackhole.md#black-001---optional-native-blackhole-package-and-preserved-provenance) | Optional native Blackhole package and preserved provenance | missing | P1 | 70–130 |
| [BLACK-002](blackhole.md#black-002---blackhole-configuration-schema-defaults-and-tolerant-reads) | Blackhole configuration schema, defaults and tolerant reads | missing | P1 | 80–150 |
| [BLACK-003](blackhole.md#black-003---environment-override-precedence-and-percent-derived-thresholds) | Environment override precedence and percent-derived thresholds | partial | P1 | 55–100 |
| [BLACK-004](blackhole.md#black-004---configuration-writes-and-environment-persistence-interaction) | Configuration writes and environment-persistence interaction | missing | P1 | 40–80 |
| [BLACK-005](blackhole.md#black-005---namespaced-blackhole-settings-ui) | Namespaced Blackhole settings UI | missing | P2 | 65–120 |
| [BLACK-006](blackhole.md#black-006---blackhole-command-parser-and-mode-presets) | /blackhole command parser and mode presets | missing | P2 | 45–90 |
| [BLACK-007](blackhole.md#black-007---message-to-normalized-block-conversion-and-source-references) | Message-to-normalized-block conversion and source references | missing | P1 | 130–230 |
| [BLACK-008](blackhole.md#black-008---text-sanitization-before-deterministic-extraction) | Text sanitization before deterministic extraction | missing | P1 | 20–45 |
| [BLACK-009](blackhole.md#black-009---thinking-noisy-tool-and-user-wrapper-filtering) | Thinking, noisy-tool and user-wrapper filtering | missing | P1 | 40–80 |
| [BLACK-010](blackhole.md#black-010---content-clipping-sentence-boundaries-and-text-search-helpers) | Content clipping, sentence boundaries and text search helpers | missing | P2 | 80–140 |
| [BLACK-011](blackhole.md#black-011---path-aliases-and-content-bearing-tool-argument-extraction) | Path aliases and content-bearing tool argument extraction | missing | P2 | 80–150 |
| [BLACK-012](blackhole.md#black-012---skill-body-collapse-with-line-and-raw-text-variants) | Skill-body collapse with line and raw-text variants | missing | P1 | 45–85 |
| [BLACK-013](blackhole.md#black-013---content-word-budgets-locale-segmentation-and-filler-removal) | Content-word budgets, locale segmentation and filler removal | missing | P1 | 100–180 |
| [BLACK-014](blackhole.md#black-014---bash-command-digest-normalization) | Bash command digest normalization | missing | P1 | 45–90 |
| [BLACK-015](blackhole.md#black-015---tool-call-one-line-summaries-and-recognized-argument-fields) | Tool-call one-line summaries and recognized argument fields | missing | P1 | 45–80 |
| [BLACK-016](blackhole.md#black-016---brief-transcript-grouping-reference-folding-and-action-caps) | Brief transcript grouping, reference folding and action caps | missing | P1 | 170–290 |
| [BLACK-017](blackhole.md#black-017---reverse-parsing-of-brief-sections-into-transcript-entries) | Reverse parsing of brief sections into transcript entries | missing | P2 | 65–120 |
| [BLACK-018](blackhole.md#black-018---genuine-user-request-rolling-section) | Genuine user-request rolling section | missing | P1 | 35–65 |
| [BLACK-019](blackhole.md#black-019---final-prose-findings-extraction) | Final-prose findings extraction | missing | P1 | 35–70 |
| [BLACK-020](blackhole.md#black-020---recent-actions-with-heuristic-error-association) | Recent actions with heuristic error association | missing | P1 | 40–80 |
| [BLACK-021](blackhole.md#black-021---outstanding-blocker-and-error-selection) | Outstanding blocker and error selection | missing | P1 | 45–85 |
| [BLACK-022](blackhole.md#black-022---file-activity-extraction-and-fresh-category-formatting) | File activity extraction and fresh-category formatting | partial | P1 | 65–120 |
| [BLACK-023](blackhole.md#black-023---heuristic-commit-message-and-hash-extraction) | Heuristic commit-message and hash extraction | missing | P1 | 80–150 |
| [BLACK-024](blackhole.md#black-024---session-goal-and-latest-scope-change-extraction) | Session-goal and latest-scope-change extraction | missing | P1 | 110–190 |
| [BLACK-025](blackhole.md#black-025---user-preference-extraction-and-goal-deduplication) | User-preference extraction and goal deduplication | missing | P1 | 45–90 |
| [BLACK-026](blackhole.md#black-026---section-layout-bounded-brief-tail-and-terminal-width-wrapping) | Section layout, bounded brief tail and terminal-width wrapping | missing | P1 | 80–150 |
| [BLACK-027](blackhole.md#black-027---historical-section-parsing-and-rolling-item-merge-policies) | Historical section parsing and rolling-item merge policies | missing | P1 | 110–200 |
| [BLACK-028](blackhole.md#black-028---original-goal-preservation-and-newest-scope-replacement-across-compactions) | Original-goal preservation and newest-scope replacement across compactions | missing | P1 | 55–100 |
| [BLACK-029](blackhole.md#black-029---cross-compaction-file-list-union-and-category-reconciliation) | Cross-compaction file-list union and category reconciliation | partial | P1 | 50–100 |
| [BLACK-030](blackhole.md#black-030---removal-of-stale-memory-legacy-observation-sections-and-recall-boilerplate) | Removal of stale memory, legacy observation sections and recall boilerplate | missing | P1 | 55–110 |
| [BLACK-031](blackhole.md#black-031---minimal-tail-compaction-hidden-message-filtering-and-cut-sentinel) | Minimal-tail compaction, hidden-message filtering and cut sentinel | partial | P1 | 90–170 |
| [BLACK-032](blackhole.md#black-032---active-goal-and-task-list-reconstruction-from-durable-custom-state) | Active goal and task-list reconstruction from durable custom state | missing | P1 | 110–200 |
| [BLACK-033](blackhole.md#black-033---mode-plan-and-verifier-reconstruction-for-compacted-context) | Mode, plan and verifier reconstruction for compacted context | missing | P1 | 60–110 |
| [BLACK-034](blackhole.md#black-034---latest-subagent-digest-and-resumable-child-references) | Latest subagent digest and resumable child references | missing | P1 | 95–180 |
| [BLACK-035](blackhole.md#black-035---background-shell-lifecycle-reconstruction-from-branch-evidence) | Background-shell lifecycle reconstruction from branch evidence | missing | P1 | 160–280 |
| [BLACK-036](blackhole.md#black-036---mcp-web-search-fetched-url-and-external-tool-usage-reconstruction) | MCP, web-search, fetched-URL and external-tool usage reconstruction | missing | P1 | 110–190 |
| [BLACK-037](blackhole.md#black-037---session-scoped-one-shot-cognee-checkpoint-recall-consumption) | Session-scoped one-shot Cognee checkpoint recall consumption | missing | P1 | 130–230 |
| [BLACK-038](blackhole.md#black-038---optional-isolated-model-refinement-with-deterministic-fallback) | Optional isolated model refinement with deterministic fallback | partial | P1 | 110–190 |
| [BLACK-039](blackhole.md#black-039---before-compaction-engine-override-and-full-summary-composition) | Before-compaction engine override and full summary composition | partial | P1 | 130–230 |
| [BLACK-040](blackhole.md#black-040---context-only-trimming-of-pre-compaction-tool-results) | Context-only trimming of pre-compaction tool results | partial | P1 | 90–160 |
| [BLACK-041](blackhole.md#black-041---boundary-triggered-pauseresume-compaction-and-missed-request-suppression) | Boundary-triggered pause/resume compaction and missed-request suppression | partial | P1 | 90–170 |

## AUTH

| ID | Contract | Status | Priority | Estimated Rust lines |
|---|---|---|---|---:|
| [AUTH-001](providers-auth.md#auth-001---credential-schema-oauth-expiry-units-and-provider-metadata) | Credential schema, OAuth expiry units and provider metadata | partial | P1 | 100–240 |
| [AUTH-002](providers-auth.md#auth-002---serialized-credential-mutations-and-cancellation-safe-file-ownership) | Serialized credential mutations and cancellation-safe file ownership | partial | P1 | 220–450 |
| [AUTH-003](providers-auth.md#auth-003---credential-read-snapshots-error-modes-and-secret-free-enumeration) | Credential read snapshots, error modes and secret-free enumeration | partial | P2 | 130–290 |
| [AUTH-004](providers-auth.md#auth-004---authoritative-request-auth-and-double-checked-oauth-refresh) | Authoritative request auth and double-checked OAuth refresh | partial | P1 | 240–510 |
| [AUTH-005](providers-auth.md#auth-005---runtime-credential-overrides-and-serialized-commitsynchronization) | Runtime credential overrides and serialized commit/synchronization | partial | P1 | 190–390 |
| [AUTH-006](providers-auth.md#auth-006---live-model-availability-generations-snapshots-and-change-notification) | Live model availability generations, snapshots and change notification | partial | P1 | 300–620 |
| [AUTH-007](providers-auth.md#auth-007---credential-blind-modelsjson-validation-and-ultra-compatibility) | Credential-blind models.json validation and ultra compatibility | partial | P1 | 200–420 |
| [AUTH-008](providers-auth.md#auth-008---provider-composition-order-model-overrides-and-extension-registration) | Provider composition order, model overrides and extension registration | partial | P1 | 330–720 |
| [AUTH-009](providers-auth.md#auth-009---configured-request-auth-header-precedence-and-execution-preparation) | Configured request auth, header precedence and execution preparation | partial | P1 | 250–530 |
| [AUTH-010](providers-auth.md#auth-010---persisted-remote-catalog-overlays-and-refresh-validators) | Persisted remote catalog overlays and refresh validators | missing | P1 | 230–480 |
| [AUTH-011](providers-auth.md#auth-011---built-in-provider-identities-endpoint-defaults-and-generated-catalog-assembly) | Built-in provider identities, endpoint defaults and generated catalog assembly | partial | P1 | 320–780 |
| [AUTH-012](providers-auth.md#auth-012---bedrock-and-vertex-credential-choices-and-ambient-readiness) | Bedrock and Vertex credential choices and ambient readiness | missing | P1 | 230–520 |
| [AUTH-013](providers-auth.md#auth-013---cloudflare-account-aware-auth-and-gateway-header-removal) | Cloudflare account-aware auth and gateway header removal | missing | P1 | 150–330 |
| [AUTH-014](providers-auth.md#auth-014---radius-dynamic-gateway-configuration-and-legacy-catalog-restoration) | Radius dynamic gateway configuration and legacy catalog restoration | missing | P1 | 160–350 |
| [AUTH-015](providers-auth.md#auth-015---typed-login-interaction-lazy-oauth-loading-and-device-polling) | Typed login interaction, lazy OAuth loading and device polling | partial | P1 | 210–460 |
| [AUTH-016](providers-auth.md#auth-016---anthropic-subscription-oauth-browsermanual-exchange-and-refresh) | Anthropic subscription OAuth browser/manual exchange and refresh | missing | P1 | 240–470 |
| [AUTH-017](providers-auth.md#auth-017---openai-codex-browser-and-headless-device-oauth-with-account-identity) | OpenAI Codex browser and headless device OAuth with account identity | missing | P1 | 290–580 |
| [AUTH-018](providers-auth.md#auth-018---github-copilot-device-grant-proxy-endpoint-and-account-model-policy) | GitHub Copilot device grant, proxy endpoint and account model policy | missing | P1 | 300–630 |
| [AUTH-019](providers-auth.md#auth-019---openrouter-one-shot-pkce-login-and-permanent-oauth-minted-api-key) | OpenRouter one-shot PKCE login and permanent OAuth-minted API key | missing | P1 | 240–480 |
| [AUTH-020](providers-auth.md#auth-020---kimi-code-device-login-bounded-request-lifetime-and-selective-refresh-retry) | Kimi Code device login, bounded request lifetime and selective refresh retry | missing | P1 | 200–360 |
| [AUTH-021](providers-auth.md#auth-021---xai-subscription-device-login-https-verification-and-refresh-token-retention) | xAI subscription device login, HTTPS verification and refresh-token retention | missing | P1 | 170–300 |
| [AUTH-022](providers-auth.md#auth-022---radius-gateway-oauth-discovery-fixed-callback-and-immediate-device-polling) | Radius gateway OAuth discovery, fixed callback and immediate device polling | missing | P1 | 250–440 |
| [AUTH-023](providers-auth.md#auth-023---provider-attribution-session-headers-and-caller-override-order) | Provider attribution, session headers and caller override order | partial | P2 | 90–180 |
| [AUTH-024](providers-auth.md#auth-024---exact-references-fuzzy-patterns-and-ordered-scoped-model-expansion) | Exact references, fuzzy patterns and ordered scoped-model expansion | partial | P1 | 220–420 |
| [AUTH-025](providers-auth.md#auth-025---cli-model-selection-authenticated-ambiguity-and-custom-id-fallback) | CLI model selection, authenticated ambiguity and custom-id fallback | partial | P1 | 170–330 |
| [AUTH-026](providers-auth.md#auth-026---initial-and-restored-model-priority-with-provider-defaults) | Initial and restored model priority with provider defaults | partial | P1 | 130–260 |
| [AUTH-027](providers-auth.md#auth-027---omni-endpoint-catalog-validation-reasoning-maps-and-route-capabilities) | Omni endpoint catalog validation, reasoning maps and route capabilities | missing | P1 | 190–350 |
| [AUTH-028](providers-auth.md#auth-028---provider-catalog-settings-submenu-and-refetch-lifecycle) | Provider catalog settings submenu and refetch lifecycle | missing | P2 | 170–300 |
| [AUTH-029](providers-auth.md#auth-029---faux-scripted-factories-model-identity-and-streaming-order) | Faux scripted factories, model identity and streaming order | partial | P1 | 200–380 |
| [AUTH-030](providers-auth.md#auth-030---faux-usage-serialization-and-prompt-cache-affinity) | Faux usage serialization and prompt-cache affinity | partial | P2 | 90–170 |
| [AUTH-031](providers-auth.md#auth-031---faux-deferred-submission-per-handle-polling-and-cancellation) | Faux deferred submission, per-handle polling and cancellation | partial | P1 | 180–340 |
| [AUTH-032](providers-auth.md#auth-032---omni-catalog-refresh-auth-bootstrap-and-local-versus-remote-refetch) | Omni catalog refresh, auth bootstrap and local versus remote refetch | missing | P1 | 170–300 |
| [AUTH-033](providers-auth.md#auth-033---complete-provider-compatibility-configuration-schema) | Complete provider compatibility configuration schema | partial | P1 | 180–360 |

## CLI

| ID | Contract | Status | Priority | Estimated Rust lines |
|---|---|---|---|---:|
| [CLI-001](cli-packaging.md#cli-001---executable-identity-stock-launcher-and-bun-bootstrap-ordering) | Executable identity, stock launcher and Bun bootstrap ordering | partial | P2 | 35–80 |
| [CLI-002](cli-packaging.md#cli-002---ice-distribution-preflight-early-agent-commands-and-builtin-factory-order) | ICE distribution preflight, early agent commands and builtin factory order | partial | P1 | 130–280 |
| [CLI-003](cli-packaging.md#cli-003---argument-token-grammar-complete-flag-surface-and-diagnostic-semantics) | Argument token grammar, complete flag surface and diagnostic semantics | partial | P1 | 180–360 |
| [CLI-004](cli-packaging.md#cli-004---run-mode-resolution-and-metadata-short-circuit-precedence) | Run-mode resolution and metadata short-circuit precedence | partial | P1 | 100–200 |
| [CLI-005](cli-packaging.md#cli-005---cwd-bound-runtime-factory-trust-staging-and-startup-service-composition) | Cwd-bound runtime factory, trust staging and startup service composition | partial | P0 | 220–430 |
| [CLI-006](cli-packaging.md#cli-006---startup-session-selection-exact-id-validation-naming-and-missing-cwd-recovery) | Startup session selection, exact-ID validation, naming and missing-cwd recovery | partial | P1 | 220–390 |
| [CLI-007](cli-packaging.md#cli-007---cli-model-thinking-fast-mode-and-tool-option-precedence) | CLI model, thinking, fast-mode and tool-option precedence | partial | P1 | 120–240 |
| [CLI-008](cli-packaging.md#cli-008---piped-stdin-ownership-complete-input-collection-and-read-failures) | Piped stdin ownership, complete-input collection and read failures | partial | P1 | 50–110 |
| [CLI-009](cli-packaging.md#cli-009---cli-file-attachments-typed-text-failures-and-image-preprocessing) | CLI file attachments, typed text failures and image preprocessing | partial | P1 | 110–230 |
| [CLI-010](cli-packaging.md#cli-010---initial-prompt-concatenation-and-remaining-message-ownership) | Initial prompt concatenation and remaining-message ownership | different-by-design | P2 | 0–25 |
| [CLI-011](cli-packaging.md#cli-011---credential-print-command-grammar-output-discipline-and-duration-parsing-defect) | Credential-print command grammar, output discipline and duration parsing defect | missing | P1 | 100–190 |
| [CLI-012](cli-packaging.md#cli-012---credential-print-provider-inference-refresh-policy-and-ambiguity-handling) | Credential-print provider inference, refresh policy and ambiguity handling | missing | P1 | 100–190 |
| [CLI-013](cli-packaging.md#cli-013---credential-startup-migration-and-recoverable-native-layout-migration) | Credential startup migration and recoverable native layout migration | partial | P1 | 130–240 |
| [CLI-014](cli-packaging.md#cli-014---session-managed-binary-prompt-directory-and-keybinding-startup-migrations) | Session, managed-binary, prompt-directory and keybinding startup migrations | partial | P2 | 160–300 |
| [CLI-015](cli-packaging.md#cli-015---official-first-run-gate-and-startup-theme-bootstrap) | Official first-run gate and startup theme bootstrap | partial | P2 | 130–250 |
| [CLI-016](cli-packaging.md#cli-016---startup-selectors-config-wrapper-and-resume-picker-terminal-lifetime) | Startup selectors, config wrapper and resume-picker terminal lifetime | partial | P2 | 100–220 |
| [CLI-017](cli-packaging.md#cli-017---available-model-listing-fuzzy-filtering-and-stable-tabular-output) | Available-model listing, fuzzy filtering and stable tabular output | partial | P2 | 65–130 |
| [CLI-018](cli-packaging.md#cli-018---exact-identity-helpers-canonical-paths-and-deliberately-limited-legacy-compatibility) | Exact identity helpers, canonical paths and deliberately limited legacy compatibility | different-by-design | P2 | 20–80 |
| [CLI-019](cli-packaging.md#cli-019---offline-normalization-startup-diagnostics-output-ownership-and-shutdown-integration) | Offline normalization, startup diagnostics, output ownership and shutdown integration | partial | P1 | 140–280 |
| [CLI-020](cli-packaging.md#cli-020---package-source-grammar-canonical-identity-and-scoped-local-path-resolution) | Package source grammar, canonical identity and scoped local-path resolution | missing | P1 | 220–360 |
| [CLI-021](cli-packaging.md#cli-021---scoped-package-settings-identity-preserving-edits-and-payload-before-config-ordering) | Scoped package settings, identity-preserving edits and payload-before-config ordering | missing | P1 | 160–260 |
| [CLI-022](cli-packaging.md#cli-022---managed-cache-layout-temporary-extension-caches-and-lexical-containment) | Managed cache layout, temporary extension caches and lexical containment | missing | P1 | 150–240 |
| [CLI-023](cli-packaging.md#cli-023---package-subprocess-environment-completion-events-timeouts-and-progress-callbacks) | Package subprocess environment, completion events, timeouts and progress callbacks | partial | P1 | 160–280 |
| [CLI-024](cli-packaging.md#cli-024---npm-pnpm-and-bun-package-installation-with-legacy-global-cache-fallback) | npm, pnpm and Bun package installation with legacy global-cache fallback | missing | P1 | 240–390 |
| [CLI-025](cli-packaging.md#cli-025---git-installation-clone-failure-cleanup-and-persistent-temporary-checkout-refresh) | Git installation, clone failure cleanup and persistent temporary checkout refresh | missing | P1 | 180–300 |
| [CLI-026](cli-packaging.md#cli-026---git-update-reconciliation-forced-remote-refs-and-incomplete-update-recovery) | Git update reconciliation, forced remote refs and incomplete-update recovery | missing | P1 | 250–420 |
| [CLI-027](cli-packaging.md#cli-027---explicit-package-update-selection-pin-policy-and-bounded-batch-concurrency) | Explicit package-update selection, pin policy and bounded batch concurrency | missing | P1 | 160–270 |
| [CLI-028](cli-packaging.md#cli-028---available-package-update-probes-remote-head-comparison-and-failure-suppression) | Available package-update probes, remote-head comparison and failure suppression | missing | P2 | 160–260 |
| [CLI-029](cli-packaging.md#cli-029---resource-manifests-convention-fallback-and-glob-expansion-by-resource-type) | Resource manifests, convention fallback and glob expansion by resource type | missing | P1 | 230–380 |
| [CLI-030](cli-packaging.md#cli-030---resource-directory-walks-ignore-files-symlinks-and-entrypoint-boundaries) | Resource directory walks, ignore files, symlinks and entrypoint boundaries | missing | P1 | 250–410 |
| [CLI-031](cli-packaging.md#cli-031---resource-filter-precedence-and-sequential-autoload-false-project-overrides) | Resource filter precedence and sequential autoload-false project overrides | missing | P1 | 180–300 |
| [CLI-032](cli-packaging.md#cli-032---effective-resource-resolution-global-first-suppression-scope-ranks-and-physical-dedup) | Effective resource resolution, global-first suppression, scope ranks and physical dedup | missing | P1 | 300–490 |
| [CLI-033](cli-packaging.md#cli-033---missing-package-source-policy-offline-short-circuiting-and-local-source-fallback) | Missing package-source policy, offline short-circuiting and local-source fallback | missing | P1 | 160–270 |
| [CLI-034](cli-packaging.md#cli-034---package-command-parsing-helperror-precedence-and-update-target-matrix) | Package-command parsing, help/error precedence and update target matrix | partial | P1 | 180–290 |
| [CLI-035](cli-packaging.md#cli-035---package-command-project-trust-and-saved-trust-only-update-policy) | Package-command project trust and saved-trust-only update policy | partial | P0 | 130–220 |
| [CLI-036](cli-packaging.md#cli-036---installremovelist-output-and-config-selector-resolutionlaunch) | Install/remove/list output and config-selector resolution/launch | partial | P1 | 130–220 |
| [CLI-037](cli-packaging.md#cli-037---explicit-model-catalog-refresh-through-update---models) | Explicit model-catalog refresh through update --models | missing | P1 | 55–95 |
| [CLI-038](cli-packaging.md#cli-038---self-update-planning-global-command-selection-and-extension-before-self-sequencing) | Self-update planning, global command selection and extension-before-self sequencing | partial | P1 | 150–250 |
| [CLI-039](cli-packaging.md#cli-039---native-project-package-links-preserve-cargo-workflow-and-close-name-containment-holes) | Native project package links: preserve Cargo workflow and close name-containment holes | partial | P0 | 100–190 |
| [CLI-040](cli-packaging.md#cli-040---native-cargo-artifact-installation-ownership-locking-and-rollback-to-preserve) | Native Cargo artifact installation, ownership locking and rollback to preserve | different-by-design | P3 | 0–0 |
| [CLI-041](cli-packaging.md#cli-041---verified-native-extension-prebuilt-distribution-and-publisher-provenance-to-preserve) | Verified native extension prebuilt distribution and publisher provenance to preserve | different-by-design | P3 | 0–0 |
| [CLI-042](cli-packaging.md#cli-042---native-startup-update-cache-per-entry-freshness-and-backend-aware-notices-to-preserve) | Native startup update cache, per-entry freshness and backend-aware notices to preserve | different-by-design | P3 | 0–0 |
| [CLI-043](cli-packaging.md#cli-043---native-self-update-source-selection-isolated-builds-and-already-staged-retry-defect) | Native self-update source selection, isolated builds and already-staged retry defect | partial | P1 | 55–110 |
| [CLI-044](cli-packaging.md#cli-044---native-executable-activation-safeguards-and-the-post-collection-output-cap-defect) | Native executable activation safeguards and the post-collection output-cap defect | partial | P1 | 80–145 |
| [CLI-045](cli-packaging.md#cli-045---native-prebuilt-cli-release-verification-and-corrected-repository-identity) | Native prebuilt CLI release verification and corrected repository identity | different-by-design | P2 | 0–0 |
| [CLI-046](cli-packaging.md#cli-046---windows-detached-native-activation-status-consumption-and-timestamp-schema-mismatch) | Windows detached native activation, status consumption and timestamp-schema mismatch | partial | P1 | 45–90 |

## MISC

| ID | Contract | Status | Priority | Estimated Rust lines |
|---|---|---|---|---:|
| [MISC-001](misc.md#misc-001---validated-release-metadata-and-quiet-automatic-version-checks) | Validated release metadata and quiet automatic version checks | partial | P2 | 80–150 |
| [MISC-002](misc.md#misc-002---windows-loaded-dependency-quarantine-and-deferred-cleanup) | Windows loaded dependency quarantine and deferred cleanup | different-by-design | P2 | 0–0 |
| [MISC-003](misc.md#misc-003---tool-result-image-identity-failure-policy-and-post-hook-normalization) | Tool-result image identity, failure policy and post-hook normalization | partial | P1 | 120–220 |
| [MISC-004](misc.md#misc-004---optional-clipboard-module-loading-and-text-read-fallback) | Optional clipboard module loading and text-read fallback | partial | P2 | 50–110 |
| [MISC-005](misc.md#misc-005---ordered-clipboard-writes-and-bounded-remote-osc-52-fallback) | Ordered clipboard writes and bounded remote OSC 52 fallback | partial | P1 | 110–200 |
| [MISC-006](misc.md#misc-006---clipboard-image-backend-order-mime-preference-and-wsl-transfer) | Clipboard image backend order, MIME preference and WSL transfer | partial | P1 | 150–260 |
| [MISC-007](misc.md#misc-007---cancellation-linking-waiter-races-and-abortable-timer-lifetime) | Cancellation linking, waiter races and abortable timer lifetime | different-by-design | P2 | 0–0 |
| [MISC-008](misc.md#misc-008---filesystem-watcher-error-callbacks-versus-native-modification-time-polling) | Filesystem watcher error callbacks versus native modification-time polling | different-by-design | P2 | 30–70 |
| [MISC-009](misc.md#misc-009---shared-yaml-frontmatter-fences-scalar-values-and-parse-errors) | Shared YAML frontmatter fences, scalar values and parse errors | partial | P2 | 55–100 |
| [MISC-010](misc.md#misc-010---optional-fd-and-rg-discovery-download-selection-and-extraction-policy) | Optional fd and rg discovery, download selection and extraction policy | different-by-design | P2 | 0–0 |
| [MISC-011](misc.md#misc-011---coding-agent-shell-selection-environment-and-shutdown-wrapper) | Coding-agent shell selection, environment and shutdown wrapper | partial | P2 | 35–80 |
| [MISC-012](misc.md#misc-012---browser-and-default-handler-process-ownership) | Browser and default-handler process ownership | partial | P2 | 20–45 |
| [MISC-013](misc.md#misc-013---changelog-parsing-version-filtering-and-release-pinned-links) | Changelog parsing, version filtering and release-pinned links | partial | P2 | 95–170 |
| [MISC-014](misc.md#misc-014---application-ansi-stripping-and-exact-control-sequence-compatibility) | Application ANSI stripping and exact control-sequence compatibility | partial | P2 | 0–0 |
| [MISC-015](misc.md#misc-015---single-shot-text-prompts-final-message-output-and-cleanup) | Single-shot text prompts, final-message output and cleanup | partial | P1 | 85–155 |
| [MISC-016](misc.md#misc-016---single-shot-json-event-framing-backpressure-and-watch-lifetime) | Single-shot JSON event framing, backpressure and watch lifetime | partial | P1 | 135–240 |
| [MISC-017](misc.md#misc-017---html-export-assembly-theme-projection-and-file-output) | HTML export assembly, theme projection and file output | partial | P1 | 180–310 |
| [MISC-018](misc.md#misc-018---offline-html-tree-navigation-filtering-deep-links-and-download) | Offline HTML tree navigation, filtering, deep links and download | missing | P2 | 540–860 |
| [MISC-019](misc.md#misc-019---html-message-markdown-image-and-built-in-tool-rendering) | HTML message, Markdown, image and built-in tool rendering | partial | P1 | 350–610 |
| [MISC-020](misc.md#misc-020---custom-tool-stateful-rendering-and-ansi-to-html-conversion) | Custom tool stateful rendering and ANSI-to-HTML conversion | missing | P2 | 180–320 |
| [MISC-021](misc.md#misc-021---llama-router-requests-sse-progress-and-loaddownload-completion) | Llama router requests, SSE progress and load/download completion | missing | P1 | 340–560 |
| [MISC-022](misc.md#misc-022---hugging-face-token-lookup-gguf-discovery-and-quantization-grouping) | Hugging Face token lookup, GGUF discovery and quantization grouping | missing | P2 | 150–250 |
| [MISC-023](misc.md#misc-023---built-in-llama-provider-login-environment-resolution-and-catalog-publication) | Built-in llama provider login, environment resolution and catalog publication | missing | P1 | 160–280 |
| [MISC-024](misc.md#misc-024---interactive-llama-model-manager-search-and-cancellation-recovery) | Interactive llama model manager, search and cancellation recovery | missing | P1 | 430–710 |
| [MISC-025](misc.md#misc-025---single-pass-html-entity-decoding-at-the-syntax-adapter-boundary) | Single-pass HTML entity decoding at the syntax adapter boundary | different-by-design | P2 | 0–0 |
| [MISC-026](misc.md#misc-026---deployment-isolation-guidance-and-documentation-navigation) | Deployment isolation guidance and documentation navigation | different-by-design | P2 | 0–0 |
| [MISC-030](misc.md#misc-030---local-claude-sdk-client-configuration-and-private-bridge-credentials) | Local Claude SDK client configuration and private bridge credentials | missing | P0 | 100–170 |
| [MISC-031](misc.md#misc-031---claude-sdk-extension-registration-model-catalog-and-status-command) | Claude SDK extension registration, model catalog and status command | missing | P1 | 80–140 |
| [MISC-032](misc.md#misc-032---claude-sdk-session-handle-isolation-reuse-and-cache-limits) | Claude SDK session-handle isolation, reuse and cache limits | missing | P1 | 160–260 |
| [MISC-033](misc.md#misc-033---claude-sdk-request-projection-protected-envelope-and-output-authority) | Claude SDK request projection, protected envelope and output authority | missing | P1 | 180–290 |
| [MISC-034](misc.md#misc-034---claude-sdk-ndjson-framing-utf-8-decoding-and-ingestion-limits) | Claude SDK NDJSON framing, UTF-8 decoding and ingestion limits | missing | P1 | 90–150 |
| [MISC-035](misc.md#misc-035---claude-sdk-terminal-turn-validation-and-retained-tool-authority) | Claude SDK terminal-turn validation and retained tool authority | missing | P0 | 150–250 |
| [MISC-036](misc.md#misc-036---claude-sdk-preview-streaming-stable-block-indices-and-final-reconciliation) | Claude SDK preview streaming, stable block indices and final reconciliation | missing | P1 | 110–190 |
| [MISC-037](misc.md#misc-037---claude-sdk-cancellation-safe-failures-and-shutdown-limits) | Claude SDK cancellation, safe failures and shutdown limits | missing | P1 | 140–240 |
| [MISC-040](misc.md#misc-040---evaluation-runner-explicit-model-defaults-and-separate-test-configuration) | Evaluation runner, explicit model defaults and separate test configuration | missing | P2 | 120–210 |
| [MISC-041](misc.md#misc-041---isolated-eval-session-construction-prompt-transformation-and-sequential-reload-steps) | Isolated eval-session construction, prompt transformation and sequential reload steps | partial | P2 | 170–280 |
| [MISC-042](misc.md#misc-042---eval-cancellation-snapshot-before-disposal-and-cleanup-error-aggregation) | Eval cancellation, snapshot-before-disposal and cleanup error aggregation | partial | P2 | 90–160 |
| [MISC-043](misc.md#misc-043---strict-prompt-completion-normalized-transcripts-and-evaluation-usage) | Strict prompt completion, normalized transcripts and evaluation usage | partial | P2 | 140–240 |
| [MISC-044](misc.md#misc-044---task-bound-sessionsource-artifacts-and-scoped-attachment-persistence) | Task-bound session/source artifacts and scoped attachment persistence | missing | P2 | 120–210 |
| [MISC-045](misc.md#misc-045---comparative-harness-tables-canonical-input-pairing-and-failure-metadata) | Comparative harness tables, canonical input pairing and failure metadata | missing | P2 | 180–300 |
| [MISC-046](misc.md#misc-046---per-test-run-index-observation-extraction-and-interrupted-run-reporting) | Per-test run index, observation extraction and interrupted-run reporting | missing | P2 | 170–280 |
| [MISC-047](misc.md#misc-047---paired-correctness-lift-telemetry-availability-and-diagnostic-terminal-summaries) | Paired correctness lift, telemetry availability and diagnostic terminal summaries | missing | P2 | 260–430 |
| [MISC-048](misc.md#misc-048---built-in-smoke-and-extension-authoring-evaluation-scenarios) | Built-in smoke and extension-authoring evaluation scenarios | missing | P2 | 150–250 |

---

Source: `README.md` current contract index. This file is a convenience index; the linked area pages are authoritative for full descriptions and evidence.
