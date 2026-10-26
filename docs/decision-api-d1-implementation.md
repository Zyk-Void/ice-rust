# Decision API D1: typed contracts and offline codecs

D1 adds typed request and response contracts for three one-shot decision
protocols in `ice-ai`. It does not add HTTP transport, provider discovery,
authentication, credentials, model registry entries, or chat stream variants.

## Public API

Use `ice_ai::decision::openai`, `ice_ai::decision::typesafe`, or
`ice_ai::decision::inception`. Each module exposes a protocol-specific
`Request`, question and answer enums, usage DTOs, `Request::validate`,
`Request::encode_json`, and `Response::decode_json(bytes, request)`. The response
decoder returns a typed `Response` only after checking it against the request.
Response fields are read through getters; the raw deserialization DTOs remain
private so a caller cannot mistake an unvalidated wire value for a successful
decision.

`Probability` and `Confidence` are separate finite 0–1 types. Confidence is
never substituted for a probability or derived from a distribution. Each
protocol has its own usage struct because the reported fields differ. The
returned model string is preserved and is not required to equal the requested
model alias.

## Supported wire shapes

| Module | Request | Answers | Supported questions |
| --- | --- | --- | --- |
| `decision::openai` | `model`, `input`, ordered `questions` array | Ordered `answers` array | `predicate`, `choice`, `score`, per-answer `refusal` |
| `decision::typesafe` | `model`, `state`, question map keyed by name | Answer map keyed by the same names | `noul`, `choice`, `score` |
| `decision::inception` | `model`, `state`, question map keyed by name | Answer map keyed by the same names | `noul`, `choice`, `score` |

OpenAI D1 input is either text or one JSON object/array encoded in a user
`input_json` part with the documented `data` field. It does not model message
history, tool calls, images, or question instruction image parts. The OpenAI
endpoint allows unnamed questions, but this library slice requires unique,
non-empty local names so every answer can be correlated safely.

TypeSafe and Inception use separate native question enums. Their `state` and
supported native instructions preserve text, object, and array values without
flattening. TypeSafe instructions may be omitted. Inception instructions keep
the distinction between omitted, explicit `null`, and a JSON value. Inception
`image_url` parts are rejected; TypeSafe Box/assets are not modeled. Inception's
guide-only `extract` question is deferred because the published create OpenAPI
discriminator contains only `noul`, `choice`, and `score`.

The protocol limits enforced in D1 are:

- OpenAI: 1–400 questions, 2–255 typed string/boolean choices, and 2–10 score
  levels. JSON input must be an object or array no deeper than 32 containers,
  at most 10,000 JSON values, and at most 256 KiB compact UTF-8.
- TypeSafe: at least one named question, at most 255 choice options, 2–10 score
  levels, at most 64 nested containers in state/instructions/criteria, and at
  most 32 MiB encoded request bytes. The public reference gives no question
  count maximum. D1 does not validate model token/context ceilings: TypeSafe's
  documented defaults depend on the selected model and allow 64k tokens for a
  whole request and 32k for state plus any one question; its optional
  long-context header is outside this offline codec.
- Inception: at least one named question, 1–255 choice options, 1–10 score
  criteria, and at most 20 MiB encoded request bytes (the documented default
  whole-request limit). The public schema gives no question count maximum. A
  local 64-container nesting bound protects D1's JSON codec; it is not an
  Inception protocol limit.

The OpenAI score range follows the current Decisions guide, which specifies
2–10 levels; the create reference itself does not state that count bound.
Model IDs remain caller-supplied opaque strings; D1 does not include provider
model catalogs or validate token budgets against a model.

## Validation behavior

The request encoder rejects invalid counts and type-specific fields before
producing bytes. The response decoder rejects malformed JSON, duplicate object
keys, trailing content, bodies above the local 32 MiB response bound, and JSON
deeper than the local bound: 64 containers for OpenAI, or 67 for native-map
responses so a legal 64-container criteria field can be echoed inside its
response envelope. Unknown DTO fields are rejected. TypeSafe and Inception
criteria limits include the enclosing map/array. Native request serialization
stops at its body limit instead of first allocating the entire oversized JSON.

Responses are checked against their original request:

- OpenAI answer count, array order, answer names, answer type, typed selected
  choice, complete unique choice distribution, score index labels, and score
  range are checked. A `refusal` remains an explicit answer variant.
- TypeSafe and Inception require exact answer map keys, matching answer
  discriminants, allowed selected choices, complete distribution keys, exact
  score index keys, a legend matching the submitted ordered criteria, and a
  score within the requested range.
- Every probability and confidence must be finite and within 0–1. Choice and
  score distributions must sum to one within a local absolute tolerance of
  `1e-6`. Scores must match their probability-weighted level index within
  `1e-6 * max(1, abs(expected))`. TypeSafe and Inception selected choices must
  have maximal probability within `1e-6`; ties are accepted. OpenAI selected
  choices are checked for membership, without assuming an undocumented argmax
  policy. These tolerances are local validation policy, not vendor precision
  guarantees; more coarsely rounded responses can be rejected. Confidence is
  preserved independently, with no invented formula.

Errors carry only static validation categories. They do not include input
state, instructions, answer bodies, question keys, or credentials. Duplicate
JSON object keys are rejected by the protocol response decoder instead of
letting `serde_json::Value` silently keep the last value. Directly deserializing
a public answer enum does not produce a validated `Response`.

## Usage

Build a request with a protocol module (`openai`, `typesafe`, or `inception`),
encode it with `Request::encode_json`, and validate a reply with
`Response::decode_json(bytes, &request)`. With the `providers` feature,
`client::DecisionClient::decide` performs both steps over HTTP. The unit tests
in each module are runnable examples built from the documented contracts.

## Primary contract references

- [OpenAI Decisions guide](https://developers.openai.com/api/docs/guides/decisions)
- [OpenAI create reference](https://developers.openai.com/api/reference/resources/decisions/methods/create)
- [OpenAI Python SDK request types](https://github.com/openai/openai-python/blob/main/src/openai/types/decision_create_params.py)
- [OpenAI Python SDK response types](https://github.com/openai/openai-python/blob/main/src/openai/types/decision.py)
- [TypeSafe System One API reference](https://docs.typesafe.ai/api/system-one)
- [TypeSafe models and context limits](https://docs.typesafe.ai/models)
- [TypeSafe Noul](https://docs.typesafe.ai/primitives/noul), [Choice](https://docs.typesafe.ai/primitives/choice), and [Score](https://docs.typesafe.ai/primitives/score)
- [Inception create reference](https://docs.inceptionlabs.ai/api-reference/decisions/create-a-decision.md)
- [Inception OpenAPI](https://api.inceptionlabs.ai/openapi.json)

D1 is an offline library contract slice only. It does not claim transport,
provider routing, configuration, credentials, or live API support.
