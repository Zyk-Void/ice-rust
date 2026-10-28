#!/usr/bin/env python3
"""Synthetic-only parser, privacy, generation and hindsight tests."""
import importlib.util
import json
from pathlib import Path
import unittest
import sys
sys.dont_write_bytecode = True

spec = importlib.util.spec_from_file_location("heldout", Path(__file__).with_name("prepare_cognee_heldout.py"))
assert spec is not None and spec.loader is not None
h = importlib.util.module_from_spec(spec)
spec.loader.exec_module(h)


class PreparationTests(unittest.TestCase):
    def normalize(self, source, objects):
        return h.normalize(source, b"\n".join(json.dumps(o).encode() for o in objects), "fixture", h.Scrubber())

    def test_claude_user_tool_envelope_and_drop(self):
        session, issues = self.normalize("claude", [
            {"type": "user", "timestamp": "2026-10-01T00:00:00Z", "message": {"role": "user", "content": "Keep synthetic fixtures local."}},
            {"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "content": "x"*4097}]}},
        ])
        self.assertEqual([t["role"] for t in session["turns"]], ["user", "tool"])
        self.assertIn("omitted", session["turns"][1]["text"])
        self.assertEqual(issues["oversize_tool_bodies"], 1)

    def test_claude_mixed_envelope(self):
        s, issues = self.normalize("claude", [{"type": "user", "message": {"role": "user", "content": [
            {"type": "text", "text": "Keep fixture context."}, {"type": "tool_result", "content": "done"}]}}])
        self.assertEqual([t["role"] for t in s["turns"]], ["user", "tool"])
        self.assertEqual(issues["mixed_user_tool_envelopes"], 1)

    def test_codex_ignores_duplicate_events_and_developer(self):
        s, _ = self.normalize("codex", [
            {"type": "session_meta", "payload": {"id": "fixture", "cwd": "/home/mewtwo/fixture"}},
            {"type": "event_msg", "payload": {"type": "user_message", "message": "duplicate"}},
            {"type": "response_item", "payload": {"type": "message", "role": "developer", "content": "hidden"}},
            {"type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "Use synthetic fixtures."}]}},
            {"type": "response_item", "payload": {"type": "function_call_output", "output": "done"}},
        ])
        self.assertEqual(s["project"], "~/fixture")
        self.assertEqual([t["role"] for t in s["turns"]], ["user", "tool"])

    def test_icers_native(self):
        s, _ = self.normalize("icers", [{"kind": "session", "id": "fixture", "cwd": "/tmp/fixture", "createdAt": 1790812800000},
            {"type": "message", "id": "u", "message": {"role": "user", "timestamp": 1790812800000, "content": "Keep fixtures synthetic."}}])
        self.assertEqual(s["session_id"], "fixture")
        self.assertEqual(s["turns"][0]["entry_id"], "u")

    def test_least_native_tool_error(self):
        s, _ = self.normalize("least", [{"type": "session", "id": "fixture", "timestamp": "2026-10-01T00:00:00Z"},
            {"type": "message", "message": {"role": "toolResult", "isError": True, "content": [{"type": "text", "text": "failed"}]}}])
        self.assertEqual(s["turns"][0]["role"], "tool")
        self.assertTrue(s["turns"][0]["is_error"])

    def test_failed_parse_retained_as_count(self):
        _, issues = h.normalize("least", b'{broken\n[]\n{}', "fixture", h.Scrubber())
        self.assertEqual(issues["failed_json_lines"], 1)
        self.assertEqual(issues["non_object_lines"], 1)

    def test_recorded_proposals_separate(self):
        s, _ = self.normalize("claude", [{"type": "assistant", "message": {"role": "assistant", "content": [{"type": "tool_use", "name": "cognee_propose_memory", "input": {"text": "Synthetic fixture.", "category": "constraint", "source_entry_id": "u"}}]}}])
        s["key"] = "fixture"
        proposals = h.generate(s)
        self.assertEqual(len(proposals), 1)
        self.assertEqual(proposals[0]["stratum"], "recorded_proposals")

    def test_scrubber_every_kind(self):
        scrubber = h.Scrubber()
        values = ["TOKEN=synthetic", "https://user:pass@example.invalid/path", "Bearer " + "A"*20,
                  "sk-"+"q"*20, "ghp_"+"z"*20, "xoxb-"+"z"*20, "a@example.invalid",
                  "/home/mewtwo/fixture", "ab"*20, "Qk"*30]
        cleaned = [scrubber.scrub(v) for v in values]
        self.assertTrue(all(a != b for a, b in zip(values, cleaned)))
        self.assertTrue(all(scrubber.counts[name] > 0 for name, _, _ in h.REDACTIONS))
        self.assertEqual(scrubber.counts["api_token"], 3)

    def test_scrubber_idempotent_unicode(self):
        scrubber = h.Scrubber()
        text = "Preserve café: /home/mewtwo/fixture and a@example.invalid"
        once = scrubber.scrub(text)
        self.assertEqual(scrubber.scrub(once), once)
        self.assertIn("café", once)

    def test_timestamp_requires_zone(self):
        self.assertIsNone(h.timestamp("2026-10-01T00:00:00"))
        self.assertEqual(h.timestamp("2026-10-01T03:00:00+03:00"), h.timestamp("2026-10-01T00:00:00Z"))

    def test_sentence_spans_are_verbatim(self):
        text = "Always preserve validation.\nDo not expose credentials! Random fixture question?"
        self.assertEqual(len(h.sentences(text)), 3)
        self.assertTrue(all(span in text for span in h.sentences(text)))

    def test_pools_balanced_and_reproducible(self):
        s = {"key": "fixture", "session_id": "fixture", "start": 1, "end": 1, "project": "/tmp/fixture", "recorded_proposals": [], "turns": [
            {"role": "user", "entry_id": "u", "text": "\n".join([f"Always preserve synthetic validator {i}." for i in range(8)]+[f"Random synthetic fixture question {i}?" for i in range(8)])},
            {"role": "assistant", "entry_id": "a", "text": "Never select assistant statements."}]}
        results = h.generate(s)
        self.assertEqual(results, h.generate(s))
        self.assertEqual(sum(c["pool"] == "signal" for c in results), 5)
        self.assertEqual(sum(c["pool"] == "random" for c in results), 5)
        self.assertTrue(all(c["turn_index"] == 0 for c in results))

    def test_matcher_restated(self):
        self.assertTrue(h.useful_match("Always preserve synthetic fixture validation.", "Please preserve synthetic fixture validation."))

    def test_matcher_negation_and_weak_overlap(self):
        self.assertFalse(h.useful_match("Never disable synthetic validation.", "Disable synthetic validation."))
        self.assertFalse(h.useful_match("Run tests.", "Run tests."))
        self.assertFalse(h.useful_match("Preserve synthetic fixture validation.", "Investigate unrelated documentation."))

    def candidate(self):
        return {"session_timestamp": 10*h.DAY, "project": "fixture", "text": "Preserve synthetic fixture validation."}

    def later(self, start, role="user", project="fixture"):
        return {"key": "later", "project": project, "start": start, "turns": [{"role": role, "timestamp": start, "text": "Preserve synthetic fixture validation."}]}

    def test_hindsight_strict_time_project_role(self):
        c = self.candidate()
        for s in (self.later(c["session_timestamp"]), self.later(11*h.DAY, project="other"), self.later(11*h.DAY, role="assistant")):
            self.assertEqual(h.label(c, [s])[0], "UNKNOWN")
        self.assertEqual(h.label(c, [self.later(11*h.DAY)])[0], "USEFUL")

    def test_unknown_and_negative_denominator(self):
        c = self.candidate()
        self.assertEqual(h.label(c, [self.later(23*h.DAY, role="tool")])[0], "UNKNOWN")
        self.assertEqual(h.label(c, [self.later(24*h.DAY, role="tool")])[0], "NOT_USEFUL")
        c["project"] = None
        self.assertEqual(h.label(c, [self.later(30*h.DAY)])[0], "UNKNOWN")

    def test_null_codex_ids_get_unique_fallbacks(self):
        s, _ = self.normalize("codex", [{"type": "response_item", "payload": {"id": None, "type": "message", "role": "user", "content": "Synthetic fixture."}} for _ in range(2)])
        self.assertEqual([t["entry_id"] for t in s["turns"]], ["turn-0", "turn-1"])

    def test_fourteen_day_boundary(self):
        c = self.candidate()
        self.assertEqual(h.label(c, [self.later(24*h.DAY)])[0], "USEFUL")
        self.assertEqual(h.label(c, [self.later(24*h.DAY+1)])[0], "NOT_USEFUL")

    def test_audit_escapes_and_hides_label(self):
        c = {"id": "fixture", "session_key": "s", "turn_index": 0, "text": "<script>private</script>", "label": "USEFUL"}
        page = h.audit_page([c], [{"key": "s", "turns": [{"role": "user", "text": "<img src=x>"}]}]).decode()
        self.assertNotIn("<script>private</script>", page)
        self.assertNotIn("USEFUL", page)
        self.assertIn("&lt;script&gt;", page)
        self.assertIn("default-src 'none'", page)


if __name__ == "__main__":
    unittest.main()
