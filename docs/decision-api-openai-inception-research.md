# OpenAI and Inception Decisions API contracts

Research date: 2026-10-26. This is a public-documentation review for the
typed Decisions API correction to the older generic structured-output plan.
No provider login, inference, credential access, configuration change, or
source-code change was performed. The requested context-mode skill was not
available in the tool or skill catalogs; research used the available
parallel-web-search workflow and first-party documentation.

## Finding

Both vendors expose a dedicated POST /v1/decisions operation. It accepts named,
typed questions over shared evidence and returns bounded answers such as a
predicate probability, a categorical choice, or an ordinal score. It is a
distinct API contract, not a chat response_format/JSON-schema feature. OpenAI
explicitly directs arbitrary schema-shaped object generation to Structured
Outputs and reserves Decisions for its fixed answer types; Inception also
documents Decisions separately from chat completions and Structured Outputs.
[OpenAI Decisions guide](https://developers.openai.com/api/docs/guides/decisions),
[Inception Decisions guide](https://docs.inceptionlabs.ai/capabilities/decisions),
[Inception documentation index](https://docs.inceptionlabs.ai/llms.txt)

The useful shared semantic core is predicate/noul, choice, and score. The wire
protocols are not interchangeable: OpenAI sends an ordered question array and
returns an ordered answer array; Inception sends a map keyed by question name
and returns an answer map under the same keys. Inception also documents an
extract extension that OpenAI does not document, but Inception's published
OpenAPI schema currently omits it.
[OpenAI create reference](https://developers.openai.com/api/reference/resources/decisions/methods/create),
[Inception create reference and OpenAPI](https://docs.inceptionlabs.ai/api-reference/decisions/create-a-decision.md),
[Inception Extract guide](https://docs.inceptionlabs.ai/capabilities/extract)

## Wire contract comparison

| Contract | OpenAI | Inception |
| --- | --- | --- |
| URL and method | POST https://api.openai.com/v1/decisions | POST https://api.inceptionlabs.ai/v1/decisions |
| Auth | Authorization: Bearer value from OPENAI_API_KEY | Authorization: Bearer value from INCEPTION_API_KEY |
| Supported model named by docs | gpt-6-luna only | mercury-decide |
| Shared evidence | input: text string or ordered message/call/output items | state: string, JSON object, or array |
| Questions | Non-empty array; each item has type, instructions, optional name, and type-specific fields | Non-empty object keyed by question name; each value has type, optional instructions, and type-specific criteria |
| Answer correlation | Array in question order; answer has name or null | Object keyed by the submitted question names |
| Success response | {model, answers: [...], usage: {...}} | {model, answers: {...}, usage: {...}} |
| Streaming | The create request schema has no stream field or endpoint-specific SSE contract; treat it as one-shot unless OpenAI publishes otherwise | Explicitly unsupported |

The endpoint, authorization, supported model, and request examples are from the
vendors' create references and guides. The response order distinction is
explicit in the OpenAI reference and the Inception response schema retains the
question names as map keys.
[OpenAI Decisions guide](https://developers.openai.com/api/docs/guides/decisions),
[OpenAI create reference](https://developers.openai.com/api/reference/resources/decisions/methods/create),
[Inception Decisions guide](https://docs.inceptionlabs.ai/capabilities/decisions),
[Inception OpenAPI](https://api.inceptionlabs.ai/openapi.json)

### OpenAI request and response

- Required request fields are model, input, and questions. The current docs say
  1–400 questions, and the only supported model is gpt-6-luna.
  [Create reference](https://developers.openai.com/api/reference/resources/decisions/methods/create)
- Input can be a string or an ordered array containing user, developer,
  system, and assistant messages, function calls and outputs, or custom-tool
  calls and outputs. Tool outputs must reference their call by call_id.
  User messages may contain text, images, and JSON; system/developer content is
  text-only; assistant content is output text/refusal; tool outputs support
  text and images. Files, audio, and item references are not supported.
  [Decisions guide](https://developers.openai.com/api/docs/guides/decisions),
  [Create reference](https://developers.openai.com/api/reference/resources/decisions/methods/create)
- A JSON object or array is sent as a user `input_json` content part with a
  `data` field, not as a different response schema. The reference limits that
  part to 32 container levels, 10,000 values, and 256 KiB of compact UTF-8
  JSON.
  [Decisions guide](https://developers.openai.com/api/docs/guides/decisions),
  [Create reference](https://developers.openai.com/api/reference/resources/decisions/methods/create)
- Images can be base64 data URLs or publicly accessible HTTP(S) URLs. Images
  can appear in shared input and, since the GA update, in question
  instructions. I found no current per-request image-count limit in the live
  guide or reference; do not copy limits from older SDK snippets without
  rechecking them.
  [Decisions guide](https://developers.openai.com/api/docs/guides/decisions),
  [Create reference](https://developers.openai.com/api/reference/resources/decisions/methods/create)
- Required question fields: type and instructions; name is optional in the
  reference, though the guide recommends unique names so answers can be
  identified. Predicate estimates a statement's truth probability; choice has
  2–255 unique {value, description?} options, where value is a string or
  boolean; score has ordered {label, description?} levels. The create
  reference does not state a score-level count bound, while the current guide
  specifies 2–10 levels; D1 follows the guide's bound.
  [Decisions guide](https://developers.openai.com/api/docs/guides/decisions),
  [Create reference](https://developers.openai.com/api/reference/resources/decisions/methods/create)
- Answers are discriminated by type: predicate has probability; choice has the
  selected typed choice, probabilities: [{value, probability}], and confidence;
  score has the numeric expected level index, indexed {value, label,
  probability} entries, and confidence; refusal has the question name or null.
  A refusal can replace any one answer while other questions still receive
  answers.
  [Create reference](https://developers.openai.com/api/reference/resources/decisions/methods/create)
- Usage includes input/output/total token counts, cached and cache-write
  input-token details, and reasoning-token details. Optional request fields in
  the current official SDK include prompt_cache_key, prompt_cache_options, and
  safety_identifier.
  [Create reference](https://developers.openai.com/api/reference/resources/decisions/methods/create),
  [official Python SDK request types](https://github.com/openai/openai-python/blob/main/src/openai/types/decision_create_params.py),
  [official Python SDK response types](https://github.com/openai/openai-python/blob/main/src/openai/types/decision.py)

### Inception request and response

- Required request fields are model, state, and a non-empty questions map. The
  documented model is mercury-decide; the model catalog associates that model
  with /v1/decisions and lists text, JSON, and image input.
  [Decisions guide](https://docs.inceptionlabs.ai/capabilities/decisions),
  [Models](https://docs.inceptionlabs.ai/get-started/models.md)
- State is a string, JSON object, or array. A question name is its map key and
  is preserved as the answer key. Questions are independently evaluated
  against the shared state. Instructions and criteria descriptions can be
  strings, JSON objects, or arrays; instructions may be omitted or null.
  [Decisions guide](https://docs.inceptionlabs.ai/capabilities/decisions),
  [Inception OpenAPI](https://api.inceptionlabs.ai/openapi.json)
- Noul has optional instructions and optional criteria.true / criteria.false
  descriptions. Choice requires a map of 1–255 string option names to
  descriptions (descriptions may be null). Score requires an ordered list of
  1–10 criteria.
  [Decisions guide](https://docs.inceptionlabs.ai/capabilities/decisions),
  [Inception OpenAPI](https://api.inceptionlabs.ai/openapi.json)
- The response has model, an answers map, and usage with input_tokens and
  output_tokens. Noul returns a noul scalar; choice returns the
  highest-probability string key, a probability map, and confidence; score
  returns an expected index, a string-indexed probability map, a legend
  mapping indices back to criteria, and confidence. The published OpenAPI
  bounds choice/score probabilities and confidence to 0–1.
  [Decisions guide](https://docs.inceptionlabs.ai/capabilities/decisions),
  [Inception OpenAPI](https://api.inceptionlabs.ai/openapi.json)
- Inception supports image parts in state or question instructions, not in
  criteria. Images must be base64 PNG, JPEG, WebP, or GIF data URLs; remote
  image URLs are unsupported. The docs specify a maximum of 10 MiB per image,
  16 images per upstream request, and a 20 MiB default whole-request body cap.
  [Decisions guide](https://docs.inceptionlabs.ai/capabilities/decisions),
  [Create reference](https://docs.inceptionlabs.ai/api-reference/decisions/create-a-decision.md)
- Inception's official separate Extract guide describes extract as a fourth
  question type on the same endpoint. It returns exact text spans with
  character offsets and per-span confidence, plus a not_stated map. It
  documents 1–26 fields, a 32,768-token cap when extraction is used, and no
  images on requests containing extract questions. This type has no documented
  OpenAI equivalent.
  [Extract guide](https://docs.inceptionlabs.ai/capabilities/extract)

## Probability and confidence semantics

- For OpenAI predicate, probability is an estimate from 0 to 1 that the stated
  condition is true. Inception noul is the probability of a positive answer
  from 0 to 1. Those concepts can share a typed PredicateProbability
  representation when the question wording and polarity are preserved.
  [OpenAI Decisions guide](https://developers.openai.com/api/docs/guides/decisions),
  [Inception Decisions guide](https://docs.inceptionlabs.ai/capabilities/decisions)
- For score, both vendors describe the returned number as the
  probability-weighted average of ordered level indices; Inception explicitly
  numbers levels starting at zero. It is a fractional expected index, not a
  chosen class. Keep the probability vector and level labels/legend alongside
  the scalar so callers do not lose the distribution.
  [OpenAI Decisions guide](https://developers.openai.com/api/docs/guides/decisions),
  [Inception Decisions guide](https://docs.inceptionlabs.ai/capabilities/decisions)
- Confidence is a distinct response field on choice/score; neither vendor's
  general Decisions contract defines a formula that equates it to a predicate
  probability or the maximum option probability. OpenAI's score example has
  max level probability 0.7 but confidence: 0.55, so code must not replace one
  with the other. Keep provider confidence as a separate optional field and
  preserve its source/question type. For Inception extract, span confidence
  specifically means confidence of the least certain token in that span.
  [OpenAI create reference](https://developers.openai.com/api/reference/resources/decisions/methods/create),
  [OpenAI Decisions guide](https://developers.openai.com/api/docs/guides/decisions),
  [Inception Extract guide](https://docs.inceptionlabs.ai/capabilities/extract)
- The docs describe probabilities/estimates but do not promise calibration.
  OpenAI recommends selecting thresholds using labeled examples from the
  application. Do not label the returned fields as calibrated risk or
  confidence scores without separate evaluation.
  [OpenAI Decisions guide](https://developers.openai.com/api/docs/guides/decisions)

## Limits, errors, and streaming

| Area | OpenAI | Inception |
| --- | --- | --- |
| Questions/request | 1–400 | At least 1; no request question-count maximum is published in the create schema |
| Choice options | 2–255, unique, values string or bool | 1–255 named string keys; descriptions can be null |
| Score levels | Ordered {label, description?} list; the guide specifies 2–10, while the create reference omits the count bound | 1–10 ordered criteria, index 0 first |
| Images | Base64 or public HTTP(S); question instructions also support images; current endpoint docs do not state an image-count cap | Base64 PNG/JPEG/WebP/GIF only; 10 MiB each, 16/request, 20 MiB default request body; not in criteria |
| Rate limits | General API/project rate limits; no Decisions-specific RPM/TPM figures found in the Decisions docs | Published API-wide per-minute tiers: Free 1,000 requests / 1M input / 100k output tokens; Pay As You Go 3,000 / 3M / 300k; Enterprise custom |
| Endpoint streaming | No stream field or Decisions SSE event contract in the current create reference/request types; do not promise it | Explicitly unsupported |
| Endpoint errors | Create reference specifies the successful Decision object but no Decisions-only status/error table; use the general API error contract and retain status/body | OpenAPI lists 401, 402, 404, 413, 422, 429, 500, 502, 503, 504; common error guide also lists 400 |

Sources: [OpenAI create reference](https://developers.openai.com/api/reference/resources/decisions/methods/create),
[OpenAI error codes](https://developers.openai.com/api/docs/guides/error-codes),
[OpenAI rate limits](https://developers.openai.com/api/docs/guides/rate-limits),
[Inception create reference and OpenAPI](https://docs.inceptionlabs.ai/api-reference/decisions/create-a-decision.md),
[Inception errors](https://docs.inceptionlabs.ai/resources/error-codes.md),
[Inception rate limits](https://docs.inceptionlabs.ai/get-started/rate-limits.md)

Inception's create OpenAPI has separate 422 shapes: request validation can use
{detail: [...]}, while an upstream service validation error can use an
{error: {...}} envelope. Its general error-code page documents 400 Invalid
Request, but the operation-specific OpenAPI response list omits 400. Preserve
the status and raw provider error detail instead of assuming one universal
Inception error body.
[Inception create reference and OpenAPI](https://docs.inceptionlabs.ai/api-reference/decisions/create-a-decision.md),
[Inception errors](https://docs.inceptionlabs.ai/resources/error-codes.md)

OpenAI's published Decisions create request has no stream parameter. The
official Python SDK exposes the typed decisions.create() operation and
response types; its generic with_streaming_response wrapper is not by itself
evidence of a Decisions SSE protocol. Inception states directly that Decisions
does not support streaming.
[OpenAI create reference](https://developers.openai.com/api/reference/resources/decisions/methods/create),
[official Python SDK Decisions resource](https://github.com/openai/openai-python/blob/main/src/openai/resources/decisions.py),
[Inception Decisions guide](https://docs.inceptionlabs.ai/capabilities/decisions)

## SDK and machine-readable schema evidence

- OpenAI's current guide and API reference are primary sources. The official
  Python SDK has generated request/response types and a
  client.decisions.create(...) method; the official Node SDK also defines
  DecisionCreateParams, Decision, and answer variants. The current Python
  source says 1–400 questions and uses bearer auth for /decisions.
  [OpenAI create reference](https://developers.openai.com/api/reference/resources/decisions/methods/create),
  [Python request types](https://github.com/openai/openai-python/blob/main/src/openai/types/decision_create_params.py),
  [Python response types](https://github.com/openai/openai-python/blob/main/src/openai/types/decision.py),
  [Python resource](https://github.com/openai/openai-python/blob/main/src/openai/resources/decisions.py),
  [Node resource and types](https://github.com/openai/openai-node/blob/main/src/resources/decisions.ts)
- Inception exposes an official OpenAPI 3.1 document at
  https://api.inceptionlabs.ai/openapi.json, and the create-reference page
  embeds that operation's schema. The official SDK page names Python and
  TypeScript packages (inceptionai), but the Decisions endpoint page
  demonstrates curl, Python urllib, and TypeScript fetch, not an SDK Decisions
  method. Whether a given SDK release exposes a typed Decisions method remains
  unverified from the docs; use the REST/OpenAPI contract as the source of
  truth until confirmed.
  [Inception OpenAPI](https://api.inceptionlabs.ai/openapi.json),
  [Inception SDKs](https://docs.inceptionlabs.ai/get-started/sdks),
  [Inception create reference](https://docs.inceptionlabs.ai/api-reference/decisions/create-a-decision)
- Additional first-party OpenAI Decisions material includes organization
  Decisions usage reporting and the native GPT-Live integration. These are
  operation-specific docs, unlike generic Structured Outputs guides.
  [Decisions usage reference](https://developers.openai.com/api/reference/resources/admin/subresources/organization/subresources/usage/methods/decisions),
  [Connect voice to Decisions](https://developers.openai.com/api/docs/guides/decisions-voice)

## Recommendation for ice-rust

Keep Decisions as a dedicated typed operation above the current chat streaming
path. A plausible shared semantic model is:

- DecisionQuestion discriminant: Predicate {id, instructions},
  Choice {id, instructions, options}, Score {id, instructions, levels}, and an
  Inception-specific Extract extension.
- DecisionAnswer discriminant: Predicate {probability_true}, Choice {selected,
  probabilities, provider_confidence?}, Score {expected_level_index,
  probabilities, provider_confidence?}, OpenAI Refusal, and Inception Extract
  {spans, not_stated}.
- Keep exact provider request and response structs at adapter boundaries and
  map only verified common semantics. This is a proposal, not a provider
  promise.

In particular:

1. Keep evidence input discriminated. OpenAI input is an ordered message/tool
   history with typed content parts; Inception state is a string or JSON value.
   Do not flatten messages, tools, images, or JSON into a chat string. Reject
   unsupported transformations explicitly.
2. Normalize correlation by a stable internal question ID, but preserve each
   wire shape: serialize IDs as OpenAI name fields and as Inception map keys;
   decode OpenAI's ordered array and Inception's keyed map without assuming
   their ordering rules match.
3. Preserve ChoiceValue as string-or-bool in the domain type. Inception only
   documents string option names, so its adapter must reject boolean values
   rather than silently stringify them. Keep each provider's option-count and
   score-level validation in its adapter.
4. Use one common predicate/choice/score question algebra, while retaining
   provider extensions for OpenAI refusals, OpenAI image/JSON/message details,
   and Inception extraction. Do not fabricate an OpenAI extract equivalent or
   an Inception refusal result.
5. Preserve the returned distribution, scalar/selected answer, provider
   confidence, model, and usage as separate fields. Never substitute
   confidence for a predicate probability, an option probability, or the score
   distribution.
6. Do not represent a Decisions request as a response_format/JSON schema or as
   streamed text. Expose a one-shot async operation (e.g. a dedicated decide
   method) and provider capability metadata for model IDs, context forms, image
   restrictions, extract, and streaming.

The shared three-question algebra and score expectation are documented by both
providers, while the adapter split follows the incompatible input/output
containers, choice value types, image restrictions, and Inception-only
extraction contract.
[OpenAI Decisions guide](https://developers.openai.com/api/docs/guides/decisions),
[OpenAI create reference](https://developers.openai.com/api/reference/resources/decisions/methods/create),
[Inception Decisions guide](https://docs.inceptionlabs.ai/capabilities/decisions),
[Inception Extract guide](https://docs.inceptionlabs.ai/capabilities/extract),
[Inception OpenAPI](https://api.inceptionlabs.ai/openapi.json)

## Offline fixture candidates

These fixtures can be built from vendor examples and schemas without API keys or
live inference:

1. OpenAI predicate request/response, including name: null in a refusal and a
   second non-refused answer to prove per-question refusal behavior.
2. Inception mixed noul + choice + score request/response, keyed by question
   name, with the documented .97, .96/.04, and 1.1 score example.
3. OpenAI choice values with both "true" and true, verifying they remain
   distinct typed values; a negative test for serializing booleans to Inception.
4. Score examples for both vendors with the full probability vector, score
   index/legend, and confidence field preserved separately.
5. Inception extract response with multiple spans, not_stated, exact exclusive-
   end offsets, and span confidence; retain a schema-discrepancy fixture
   showing this documented request is absent from OpenAPI's discriminator.
6. Input serialization fixtures for OpenAI input_json, image data URL and
   hosted image reference, message/tool history, plus Inception JSON state and
   base64-only image parts.
7. Error fixtures: Inception 422 validation detail, 422 upstream error, 413 and
   429; OpenAI generic bearer/API error envelope with raw status/body.
8. Boundary validation fixtures for 400 OpenAI questions, 255 choice options,
   Inception 10 score levels, max 16 images/20 MiB body, and extraction's
   26 fields / 32,768-token request limit. Do not make oversized live requests
   to test these.
9. Assert no parsing path normalizes probability totals, equates confidence
   with max probability, maps Inception question-map iteration order to
   OpenAI's answer-array order, or converts provider refusals into HTTP errors.

The concrete example values and validation limits above come from the
first-party guides and schemas; fixtures should record them as protocol samples,
not as claims that a live model has been exercised.
[OpenAI Decisions guide](https://developers.openai.com/api/docs/guides/decisions),
[OpenAI create reference](https://developers.openai.com/api/reference/resources/decisions/methods/create),
[Inception create reference](https://docs.inceptionlabs.ai/api-reference/decisions/create-a-decision.md),
[Inception Extract guide](https://docs.inceptionlabs.ai/capabilities/extract)

## Open questions for implementation planning

- Inception's Extract guide and model catalog document extract, but the
  operation's published OpenAPI discriminator and response union contain only
  noul, choice, and score. Confirm the service/schema version before finalizing
  generated request types.
  [Inception create reference](https://docs.inceptionlabs.ai/api-reference/decisions/create-a-decision.md),
  [Inception Extract guide](https://docs.inceptionlabs.ai/capabilities/extract),
  [Models](https://docs.inceptionlabs.ai/get-started/models.md)
- The OpenAI create reference exposes `score.levels` but publishes no count
  bound; the current guide specifies 2–10. Inception constrains score criteria
  to 1–10. D1 follows each protocol's current guide/schema limit.
  [OpenAI create reference](https://developers.openai.com/api/reference/resources/decisions/methods/create),
  [Inception OpenAPI](https://api.inceptionlabs.ai/openapi.json)
- Neither provider documents a general calibration guarantee or a formula for
  choice/score confidence; preserve raw fields and leave threshold policy to a
  separately evaluated application.
  [OpenAI Decisions guide](https://developers.openai.com/api/docs/guides/decisions),
  [Inception Decisions guide](https://docs.inceptionlabs.ai/capabilities/decisions)
- Inception's SDK docs publish package names but don't show a Decisions method;
  check the exact chosen package version before making it an implementation
  dependency.
  [Inception SDKs](https://docs.inceptionlabs.ai/get-started/sdks),
  [Inception create reference](https://docs.inceptionlabs.ai/api-reference/decisions/create-a-decision)
- OpenAI's endpoint reference does not enumerate a Decisions-specific error
  table. Use general API error handling, retain the provider body, and avoid
  baking in an unverified exhaustive status enum.
  [OpenAI create reference](https://developers.openai.com/api/reference/resources/decisions/methods/create),
  [OpenAI error codes](https://developers.openai.com/api/docs/guides/error-codes)

