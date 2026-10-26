# Jev / TypeSafe AI Decision API research

Researched 2026-10-26 from public documentation only. No account was accessed and no live API request or credential was used.

## Finding

The supplied [Decision APIs article](https://jevtypesafeai.com/decision-api) is hosted by JevTypeSafeAI.com, an independent service. Its own [API docs](https://jevtypesafeai.com/docs) say the site is not affiliated with or endorsed by TypeSafe AI. The site documents a gateway endpoint and its own <code>jv_live_</code> keys. TypeSafe's native API is documented separately at <code>https://api.typesafe.ai/v1/systemone</code>, with Bearer-token authentication and a <code>TYPESAFE_API_KEY</code> environment variable in its quick start. Do not treat <code>jv_live_</code> as a documented TypeSafe-native key prefix. [TypeSafe API reference](https://docs.typesafe.ai/api/system-one), [TypeSafe quick start](https://docs.typesafe.ai/introduction/quickstart), [Jev gateway docs](https://jevtypesafeai.com/docs)

## Endpoint and authentication boundary

| Surface | Endpoint and contract | Authentication / ownership |
| --- | --- | --- |
| TypeSafe native System One | <code>POST https://api.typesafe.ai/v1/systemone</code>; request body uses <code>model</code>, <code>state</code>, and <code>questions</code>. | <code>Authorization: Bearer &lt;token&gt;</code>. TypeSafe says to get an API key from the TypeSafe console and suggests <code>TYPESAFE_API_KEY</code> for local environment configuration. [API reference](https://docs.typesafe.ai/api/system-one), [quick start](https://docs.typesafe.ai/introduction/quickstart) |
| JevTypeSafeAI hosted gateway | <code>POST https://jevtypesafeai.com/api/v1/decide</code>; its request resembles the System One body and it says it forwards that body to TypeSafe. | The independent site's own <code>jv_live_</code> Bearer key, obtained through its pricing page after sign-in and prepayment. The site's footer disclaims TypeSafe affiliation. [Jev API page](https://jevtypesafeai.com/jev/api), [gateway API docs](https://jevtypesafeai.com/docs) |
| JevTypeSafeAI unified multi-model gateway | <code>POST https://jevtypesafeai.com/api/v1/decisions</code>; its docs add a model selector and <code>provider</code>, <code>provider_data</code>, and gateway billing fields. | Same independent gateway key. This is a separate aggregator contract for Jev, Clef, Clef Flash, and OpenAI Decisions; it is not TypeSafe's native <code>/v1/systemone</code>. [Decision models API](https://jevtypesafeai.com/docs/decisions) |

The native HTTP API requires a Bearer header. The public API reference calls the credential a token; the quick start calls it an API key. The TypeSafe console's public key page redirects to a sign-in page offering Google or email. This is interactive account/key provisioning; the native API docs do not describe an OAuth authorization-code flow or a browser login exchange for API calls. The native key's textual prefix is not specified in the cited TypeSafe docs, so treat it as an opaque secret Bearer credential and do not assume a JWT format. [TypeSafe API reference](https://docs.typesafe.ai/api/system-one), [quick start](https://docs.typesafe.ai/introduction/quickstart), [TypeSafe console sign-in](https://console.typesafe.ai/keys)

## Native request and response

The official API reference marks <code>model</code>, <code>state</code>, and <code>questions</code> as required. <code>state</code> may be a string, object, or array. <code>questions</code> is an object map keyed by caller-chosen question names; each answer is returned under that same key. <code>model</code> is a model name or alias. <code>GET https://api.typesafe.ai/v1/models</code> lists names available to the account; versioned IDs are also accepted. The response reports the model that actually answered, which may differ from an alias such as <code>jev-latest</code>. [System One API reference](https://docs.typesafe.ai/api/system-one), [Models reference](https://docs.typesafe.ai/models)

The API example below is transcribed from TypeSafe's reference:

    POST https://api.typesafe.ai/v1/systemone
    Authorization: Bearer <token>
    Content-Type: application/json

    {
      "model": "jev-latest",
      "state": "I was charged twice for my subscription this month. Please refund the extra charge.",
      "questions": {
        "billing": {
          "type": "noul",
          "instructions": "Is this message about billing?"
        }
      }
    }

The reference's example 200 response is:

    {
      "model": "jev-latest",
      "answers": {
        "billing": {
          "type": "noul",
          "noul": 0.98
        }
      },
      "usage": {
        "input_tokens": 120,
        "output_tokens": 12
      }
    }

Each answer has the same type as its question. <code>usage</code> reports input and output token counts. [System One API reference](https://docs.typesafe.ai/api/system-one)

### Question and answer variants

| Type | Request shape | Native answer semantics |
| --- | --- | --- |
| <code>choice</code> | <code>type: "choice"</code>, <code>instructions</code>, and <code>criteria</code> as a map of option names to descriptions. TypeSafe documents up to 255 options. | <code>choice</code> is the highest-probability option; <code>probabilities</code> covers every option and sums to 1; <code>confidence</code> is a separate 0–1 measure derived from how spread out the probabilities are. [Choice docs](https://docs.typesafe.ai/primitives/choice) |
| <code>score</code> | <code>type: "score"</code>, <code>instructions</code>, and <code>criteria</code> as an ordered array of level descriptions. TypeSafe says 2–10 levels. | <code>score</code> is the probability-weighted mean of the zero-based level indices and may be fractional. The response includes per-level <code>probabilities</code>, <code>confidence</code>, and a <code>legend</code> mapping indices to descriptions. [Score docs](https://docs.typesafe.ai/primitives/score) |
| <code>noul</code> | <code>type: "noul"</code>, <code>instructions</code>, and optional <code>criteria</code> with <code>true</code> and <code>false</code> descriptions. | <code>noul</code> is a single probability from 0 to 1 that the yes/true answer applies; values near 0.5 indicate uncertainty. The documented Noul response has no separate confidence field. [Noul docs](https://docs.typesafe.ai/primitives/noul) |
| <code>box</code> | <code>type: "box"</code> and <code>instructions</code> naming what to find and which image. Images are supplied through <code>assets</code>; requests need <code>TypeSafe-Danger-Zone-Images: true</code>. | <code>box</code> has <code>top</code>, <code>bottom</code>, <code>left</code>, and <code>right</code> edges as 0–1 fractions plus <code>confidence</code>. It always returns a box, including when the target is absent; TypeSafe recommends a companion Noul check. Images and Box are explicitly marked not production-ready and subject to change. [Box docs](https://docs.typesafe.ai/primitives/box), [API reference](https://docs.typesafe.ai/api/system-one) |

The System One reference marks each question's <code>type</code> as required but
does not mark <code>instructions</code> required. D1 therefore allows native
instructions to be omitted and preserves their documented string/object/array
forms when present. Choice descriptions may be null, and Choice/Score docs also
show structured object descriptions. [System One API reference](https://docs.typesafe.ai/api/system-one), [Choice](https://docs.typesafe.ai/primitives/choice), [Score](https://docs.typesafe.ai/primitives/score)

The answer contract is structured decision data, not generated prose: typed answers are keyed by question name, with probabilities and confidence where that primitive defines them. TypeSafe describes Jev as making decisions from shared state and typed questions rather than generating text. [Introduction](https://docs.typesafe.ai/introduction), [State](https://docs.typesafe.ai/concepts/state)

## Multiple questions, streaming, and model selection

One request evaluates a single <code>state</code> against any number of named questions. Questions see the same state and are evaluated independently in parallel. TypeSafe documents no separate question-count limit, provided the request fits the context limits. The native docs do not describe a separate multi-state batch endpoint. [State](https://docs.typesafe.ai/concepts/state), [Models](https://docs.typesafe.ai/models)

The native API reference documents a POST returning a JSON success response and no streaming option. LiteLLM's TypeSafe pass-through guide independently states that streaming is not offered by the TypeSafe API. The public contract therefore supports a single decision result per call; no token-delta stream contract was found. [System One API reference](https://docs.typesafe.ai/api/system-one), [LiteLLM TypeSafe pass-through](https://docs.litellm.ai/docs/pass_through/typesafe)

For direct HTTP calls, TypeSafe's API reference marks <code>model</code> required. The official SDK can set a default model, and <code>jev-latest</code> is the documented moving alias; the current Models page maps it to <code>jev-1.14.0</code> and says the response reports the version that answered. Pin a version when stable thresholds matter. The independent Jev gateway says its own <code>model</code> field is optional and defaults to <code>jev-latest</code>; that gateway behavior should not be copied into the native HTTP contract. [API reference](https://docs.typesafe.ai/api/system-one), [Models](https://docs.typesafe.ai/models), [gateway docs](https://jevtypesafeai.com/docs)

## Limits and errors

TypeSafe's current documented Jev models have a default context limit of 64k tokens for the whole request and 32k for the state plus any one question. The <code>TypeSafe-Danger-Zone-Long-Context: true</code> header raises both limits to 262k; TypeSafe says long-context calls are slower and less accurate than staying within the defaults. [Models](https://docs.typesafe.ai/models)

Rate limits are per organization and are measured in tokens per second and requests per second. The public docs do not give a fixed numeric rate; current account limits are in the developer console. Exceeding either limit returns 429. TypeSafe's request-rules page also lists 529 for temporary overload. It recommends exponential backoff for 429 and 529; official SDKs retry by default and honor <code>retry-after</code> when present. [Models](https://docs.typesafe.ai/models), [Request rules and errors](https://docs.typesafe.ai/api/errors)

Other native request constraints: body size at most 32 MiB; UTF-8 JSON without BOM; at most 64 nested object/array levels in <code>state</code>, <code>instructions</code>, and <code>criteria</code>; finite numeric values only. The body may use gzip, x-gzip, or zstd compression, one layer only, with zstd window at most 8 MiB. The 32 MiB cap applies to compressed and decompressed bodies. [Request rules and errors](https://docs.typesafe.ai/api/errors)

| Native status | Documented meaning |
| --- | --- |
| 400 | Request rule failure, malformed compressed body, or <code>max_tokens_exceeded</code> context length. |
| 401 | Missing or invalid API key. |
| 413 | Body exceeds 32 MiB. |
| 415 | Unsupported or multiply layered <code>Content-Encoding</code>. |
| 422 | Body validation failure, such as a missing required field or malformed question; details identify the offending field. |
| 429 | Rate limit exceeded. |
| 529 | Temporary TypeSafe overload. |

Errors use a JSON body describing the problem. The cited native docs do not define a single detailed JSON error-envelope schema. The independent Jev gateway documents additional gateway statuses (402, 403, 404, and 502); these should not be added to the native TypeSafe status contract without native evidence. [TypeSafe request rules and errors](https://docs.typesafe.ai/api/errors), [gateway error docs](https://jevtypesafeai.com/docs)

## Relevant ecosystem references

- TypeSafe's official [quick start](https://docs.typesafe.ai/introduction/quickstart) covers Python and JavaScript/TypeScript SDK clients and the direct HTTP call. The Jev ecosystem page links the [official TypeScript SDK repository](https://github.com/typesafe-ai/typesafe-sdk-js).
- JevTypeSafeAI.com's [ecosystem page](https://jevtypesafeai.com/ecosystem) lists MCP servers and other community projects. It is a discovery list on the independent gateway site, not TypeSafe's API specification.
- [LiteLLM's TypeSafe pass-through docs](https://docs.litellm.ai/docs/pass_through/typesafe) show a <code>/typesafe/v1/systemone</code> proxy route that forwards the System One request shape. LiteLLM separately documents [its /v1/systemone endpoint](https://docs.litellm.ai/docs/decisions/systemone_format) and [its /v1/decisions endpoint](https://docs.litellm.ai/docs/decisions/openai_format); the latter uses the OpenAI Decisions format (<code>input</code> plus an array of questions with types such as <code>predicate</code>). Those proxy routes and formats are distinct from TypeSafe's native endpoint and should not be confused with generic JSON-schema output or <code>/chat/completions</code>.
- [Pydantic AI's TypeSafe page](https://pydantic.dev/docs/ai/models/typesafe/) describes <code>TypeSafeModel</code> as a subclass of its <code>DecisionModel</code> and says Jev answers typed questions rather than writing text. This is a third-party adapter, but a useful ecosystem example of keeping decision-model semantics distinct from ordinary chat generation.

## Implications for a provider-neutral Decisions seam

The wire contract supports a separate synchronous decision operation with a JSON-valued state, a named map of typed questions, typed answers keyed by those names, the resolved model, and usage counts. A provider-neutral abstraction should preserve the discriminated answer variants (<code>choice</code>, <code>score</code>, <code>noul</code>, optionally <code>box</code>) and their probability/confidence data instead of converting them to arbitrary generated JSON. This is a design inference from the native request and answer contract. [System One API reference](https://docs.typesafe.ai/api/system-one), [Choice](https://docs.typesafe.ai/primitives/choice), [Score](https://docs.typesafe.ai/primitives/score), [Noul](https://docs.typesafe.ai/primitives/noul)

Keep this operation separate from the chat provider stream: TypeSafe's public API is a request/response decision endpoint with no documented streaming contract. Represent auth as an opaque Bearer API key; do not assume JWT or import the independent gateway's <code>jv_live_</code> prefix into native TypeSafe configuration. Treat Box/images as an opt-in preview capability. [System One API reference](https://docs.typesafe.ai/api/system-one), [LiteLLM TypeSafe pass-through](https://docs.litellm.ai/docs/pass_through/typesafe), [gateway ownership notice](https://jevtypesafeai.com/docs)

## Does generic JSON-schema output cover this?

No. Generic JSON-schema output constrains the shape of generated content. The native System One API accepts a state and explicit typed questions, then returns primitive-specific answers and probabilities. That generic work does not implement the TypeSafe endpoint, key boundary, question types, answer distributions, model selection, or its error and usage contract. Shared JSON serialization may still be reusable, but this is a separate API capability. [TypeSafe Introduction](https://docs.typesafe.ai/introduction), [System One API reference](https://docs.typesafe.ai/api/system-one)

## Verified vs. unresolved

Verified from TypeSafe's public docs: native endpoint and Bearer auth; API-key provisioning location; request and response shape; question/answer types; multi-question evaluation; model discovery; context/body/rate constraints; and documented HTTP statuses.

Not specified by the native public docs reviewed: the API key's textual prefix or exact token format; a browser OAuth flow for API requests; a fixed organization rate-limit number; a complete JSON error body schema; a native cross-state bulk endpoint; or a streaming response contract. <code>jv_live_</code>, the gateway's billing fields, and its additional status codes belong to the independent JevTypeSafeAI service.

## Sources

- [System One API reference](https://docs.typesafe.ai/api/system-one)
- [TypeSafe request rules and errors](https://docs.typesafe.ai/api/errors)
- [TypeSafe models](https://docs.typesafe.ai/models)
- [TypeSafe quick start](https://docs.typesafe.ai/introduction/quickstart)
- [TypeSafe introduction](https://docs.typesafe.ai/introduction)
- [TypeSafe state](https://docs.typesafe.ai/concepts/state)
- [TypeSafe Choice primitive](https://docs.typesafe.ai/primitives/choice)
- [TypeSafe Score primitive](https://docs.typesafe.ai/primitives/score)
- [TypeSafe Noul primitive](https://docs.typesafe.ai/primitives/noul)
- [TypeSafe Box primitive](https://docs.typesafe.ai/primitives/box)
- [TypeSafe console key sign-in](https://console.typesafe.ai/keys)
- [JevTypeSafeAI Decision APIs article](https://jevtypesafeai.com/decision-api)
- [JevTypeSafeAI Jev API](https://jevtypesafeai.com/jev/api)
- [JevTypeSafeAI API docs](https://jevtypesafeai.com/docs)
- [JevTypeSafeAI Decision models API](https://jevtypesafeai.com/docs/decisions)
- [JevTypeSafeAI ecosystem list](https://jevtypesafeai.com/ecosystem)
- [TypeSafe official JavaScript SDK repository](https://github.com/typesafe-ai/typesafe-sdk-js)
- [LiteLLM TypeSafe pass-through](https://docs.litellm.ai/docs/pass_through/typesafe)
- [LiteLLM System One format](https://docs.litellm.ai/docs/decisions/systemone_format)
- [LiteLLM OpenAI Decisions format](https://docs.litellm.ai/docs/decisions/openai_format)
- [Pydantic AI TypeSafe model](https://pydantic.dev/docs/ai/models/typesafe/)
