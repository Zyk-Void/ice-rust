#!/usr/bin/env python3
"""Local-only M5.4 preparation. Run with python3 -I; never imports provider code.

prepare writes scrubbed sessions and native replay input; freeze requires the
ignored Rust test's decisions. All private artifacts stay in the fixed target.
"""
import argparse
import collections
import datetime as dt
import hashlib
import html
import json
import os
from pathlib import Path
import re
import stat

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "target/cognee-remaining-validation/heldout"
SEED = 20261028
DAY = 86400000
CUTOFF = "2026-10-20T00:00:00"
SIGNALS = [
    r"\b(?:always|never|don't|do not|prefer|please use|we decided|from now on|remember)\b",
    r"^(?:please\s+)?(?:use|keep|avoid|ensure|run|test|do|don't|stop|read|write|preserve|follow|must|only)\b",
    r"\b(?:must|must not|should not|required|constraint|instead of|rather than)\b",
]
SPLITTER = r"[^\n.!?]+(?:[.!?](?=\s|$)|$)"
STOP = set("the and for that this with from have has had are was were will would should could can not don't do does did please use using only also then than into your you our their they them its it's but all any when where which what how why who been being each these those must always never now same more less some such need want make".split())
RULES = {
    "version": 1, "seed": SEED, "cutoff_local": CUTOFF,
    "signal_patterns": SIGNALS, "sentence_regex": SPLITTER,
    "split_timestamp": "session START; candidate and hindsight reference timestamp is session END",
    "sentence_rule": "verbatim regex matches stripped only at edges; newline and .!? delimit; no sentence fragments split to fit budget",
    "eligible_chars": [8, 300], "signal_max_per_session": 5,
    "sampling": "sorted source paths; chronological sessions; seeded SHA256-ranked sentences without replacement; random pool equals signal count when available; shortages retained",
    "kind_hint": "preference if prefer; decision if we decided; otherwise constraint, including random pool",
    "matcher": {
        "tokens": "casefold Unicode alphanumeric/underscore tokens, length >=3, minus frozen stop words",
        "stop_words": sorted(STOP), "minimum_shared": 3,
        "candidate_token_coverage": 0.70, "key_terms": "tokens length >=6 or containing underscore/digit; at least two shared",
        "negation": "equal presence of never/not/don't/avoid/mustn't/cannot in each sentence",
        "evidence": "later USER sentences only; no assistant/tool evidence or LLM; restatement proxy, not semantic truth",
        "time": "candidate session END; later session START strictly greater; evidence timestamp strictly greater and <=14 days; exact scrubbed known cwd required",
        "negative": "no match and later known same-project session START >= candidate END+14 days; otherwise UNKNOWN",
    },
    "audit": "100 held-out candidates, 50 USEFUL and 50 NOT_USEFUL if possible; fill shortages uniformly from all remaining including UNKNOWN; labels hidden",
    "tool_body_limit_bytes": 4096,
}
REDACTIONS = [
    ("env_assignment", re.compile(r"(?m)^\s*(?:export\s+)?[A-Za-z_][A-Za-z0-9_]*=[^\n]*$"), "[REDACTED_ENV]"),
    ("url_credentials", re.compile(r"(?i)(https?://)[^\s/@]+(?::[^\s/@]*)?@"), r"\1[REDACTED_CREDENTIALS]@"),
    ("bearer", re.compile(r"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]+"), "Bearer [REDACTED]"),
    ("api_token", re.compile(r"\b(?:sk-|ghp_|github_pat_|xox[baprs]-?)[A-Za-z0-9_-]{8,}"), "[REDACTED_TOKEN]"),
    ("email", re.compile(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b"), "[REDACTED_EMAIL]"),
    ("home_path", re.compile(r"/home/mewtwo(?=/|\b)"), "~"),
    ("long_hex", re.compile(r"\b[0-9a-fA-F]{32,}\b"), "[REDACTED_HEX]"),
    ("long_base64", re.compile(r"(?<![\w/+])[A-Za-z0-9_+/=-]{48,}(?![\w/+])"), "[REDACTED_LONG_RUN]"),
]


def digest(value):
    return hashlib.sha256(value).hexdigest()


def encoded(value):
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode()


def private_dir(out):
    """Create the private output directory; no component below ROOT may be a link."""
    for part in (out, *out.parents):
        if part == ROOT:
            break
        if part.is_symlink():
            raise ValueError(f"private output path must not contain a symlink: {part.name}")
    out.mkdir(parents=True, exist_ok=True, mode=0o700)
    if out.is_symlink():
        raise ValueError("private output directory must not be a symlink")
    os.chmod(out, 0o700)


def private_write(name, value, raw=False):
    private_dir(OUT)
    path = OUT / name
    # Never follow a planted output symlink or overwrite a previous freeze.
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "wb") as stream:
        stream.write(value if raw else encoded(value))
    return path


class Scrubber:
    def __init__(self):
        self.counts = collections.Counter({name: 0 for name, _, _ in REDACTIONS})

    def scrub(self, text):
        for name, pattern, replacement in REDACTIONS:
            text, n = pattern.subn(replacement, text)
            self.counts[name] += n
        return text


def timestamp(value):
    if isinstance(value, (float, int)):
        return int(value if value > 100000000000 else value * 1000)
    if isinstance(value, str):
        try:
            parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
            if parsed.tzinfo is None:
                return None  # Never silently assign a timezone to source history.
            return int(parsed.timestamp() * 1000)
        except ValueError:
            return None
    return None


def text_content(value):
    if isinstance(value, str):
        return value
    if isinstance(value, list):
        return "\n".join(text_content(x) for x in value if isinstance(x, (dict, str)))
    if isinstance(value, dict):
        if value.get("type") in ("text", "input_text", "output_text"):
            return str(value.get("text", ""))
        if value.get("type") == "tool_result":
            return text_content(value.get("content", ""))
    return ""


def native_record(obj):
    if obj.get("type") == "message" and isinstance(obj.get("message"), dict):
        return obj["message"]
    return None


def claude_record(obj):
    if obj.get("type") in ("user", "assistant"):
        return obj.get("message")
    return None


def codex_record(obj):
    if obj.get("type") != "response_item":
        return None  # event_msg duplicates response_item user/assistant text.
    payload = obj.get("payload", {})
    if payload.get("type") == "message":
        return payload
    if payload.get("type") in ("function_call_output", "custom_tool_call_output"):
        return {"role": "tool", "content": payload.get("output", ""), "id": payload.get("call_id")}
    if payload.get("type") in ("function_call", "custom_tool_call"):
        return {"role": "assistant", "content": [], "calls": [payload]}
    return None


def icers_record(obj):
    return native_record(obj)


def least_record(obj):
    return native_record(obj)


PARSERS = {"claude": claude_record, "codex": codex_record, "icers": icers_record, "least": least_record}


def sentences(text):
    return [m.group().strip() for m in re.finditer(SPLITTER, text) if m.group().strip()]


def tokens(text):
    return {w for w in re.findall(r"\w+", text.casefold()) if len(w) >= 3 and w not in STOP}


def useful_match(candidate, later):
    a, b = tokens(candidate), tokens(later)
    shared = a & b
    keys = {w for w in shared if len(w) >= 6 or "_" in w or any(c.isdigit() for c in w)}
    negative = lambda x: bool(re.search(r"\b(?:never|not|don't|avoid|mustn't|cannot)\b", x.casefold()))
    return (len(shared) >= 3 and len(shared) * 10 >= len(a) * 7
            and len(keys) >= 2 and negative(candidate) == negative(later))


def ranked(items, salt):
    return sorted(items, key=lambda x: digest(f"{SEED}:{salt}:{json.dumps(x, sort_keys=True)}".encode()))


def allowed_files(root):
    if Path(root).is_symlink():
        return
    for base, dirs, files in os.walk(root, followlinks=False):
        dirs[:] = sorted(d for d in dirs if d not in {".ice", ".git", "node_modules", "target"}
                         and not Path(base, d).is_symlink())
        for name in sorted(files):
            path = Path(base, name)
            if (name.endswith(".jsonl") and not name.startswith((".env", "auth.", "api_key."))
                    and not path.is_symlink() and path.is_file()):
                yield path


def normalize(source, raw, file_id, scrubber):
    turns, proposals, issues = [], [], collections.Counter()
    content_hasher = hashlib.sha256()
    project, session_id = None, file_id
    times = []
    for line in raw.splitlines():
        try:
            obj = json.loads(line)
        except (ValueError, UnicodeError):
            issues["failed_json_lines"] += 1
            continue
        if not isinstance(obj, dict):
            issues["non_object_lines"] += 1
            continue
        payload = obj.get("payload", {})
        if not isinstance(payload, dict):
            payload = {}
        if obj.get("type") == "session_meta":
            session_id = str(payload.get("id", session_id))
            project = payload.get("cwd", project)
        session_id = str(obj.get("sessionId", session_id))
        if obj.get("type") == "session" or (obj.get("kind") == "session"):
            session_id = str(obj.get("id", session_id))
        project = obj.get("cwd", project)
        if obj.get("type") == "turn_context":
            project = payload.get("cwd", project)
        at = timestamp(obj.get("timestamp", obj.get("createdAt")))
        if at is not None:
            times.append(at)
        message = PARSERS[source](obj)
        if not isinstance(message, dict):
            continue
        at = timestamp(message.get("timestamp")) or at
        if at is not None:
            times.append(at)
        role = message.get("role")
        if role == "toolResult":
            role = "tool"
        if role not in ("user", "assistant", "tool"):
            continue
        content = message.get("content", "")
        blocks = content if isinstance(content, list) else []
        tool_blocks = [b for b in blocks if isinstance(b, dict) and b.get("type") == "tool_result"]
        if role == "user" and tool_blocks:
            # Claude wraps tool results in a user envelope, not an actual user turn.
            non_tool = [b for b in blocks if b not in tool_blocks]
            if text_content(non_tool).strip():
                issues["mixed_user_tool_envelopes"] += 1
                turns.append({"role": "user", "timestamp": at, "text": scrubber.scrub(text_content(non_tool)), "entry_id": str(obj.get("uuid", obj.get("id", len(turns))))})
            content = tool_blocks
            role = "tool"
        body = text_content(content)
        content_hasher.update(encoded([role, body]))
        if role == "tool" and len(body.encode()) > 4096:
            body = "[tool output omitted: exceeds 4096 bytes]"
            issues["oversize_tool_bodies"] += 1
        entry_id = scrubber.scrub(str(obj.get("uuid") or obj.get("id") or message.get("id") or f"turn-{len(turns)}"))
        turn = {"role": role, "timestamp": at, "text": scrubber.scrub(body), "entry_id": entry_id,
                "is_error": bool(message.get("isError", message.get("is_error", False))) or any(b.get("is_error", False) for b in tool_blocks)}
        if at is None:
            issues["turns_missing_timestamp"] += 1
        turns.append(turn)
        calls = message.get("calls", []) + [b for b in blocks if isinstance(b, dict) and b.get("type") in ("tool_use", "toolCall")]
        for call in calls:
            if call.get("name") != "cognee_propose_memory":
                continue
            args = call.get("input", call.get("arguments", {}))
            if isinstance(args, str):
                try:
                    args = json.loads(args)
                except ValueError:
                    args = {}
            if not isinstance(args, dict):
                args = {}
            proposals.append({"turn_index": len(turns)-1, "args": {k: scrubber.scrub(str(args[k])) for k in ("text", "category", "source_entry_id", "verified_by") if k in args}})
    # Array order is the source order; timestamp anomalies are counted, not repaired.
    issues["timestamp_regressions"] = sum(a["timestamp"] > b["timestamp"] for a, b in zip(turns, turns[1:]) if a["timestamp"] is not None and b["timestamp"] is not None)
    return {"source": source, "session_id": scrubber.scrub(session_id), "project": scrubber.scrub(project) if isinstance(project, str) else None,
            "start": min(times) if times else None, "end": max(times) if times else None,
            "turns": turns, "recorded_proposals": proposals, "content_sha256": content_hasher.hexdigest()}, issues


def generate(session):
    pool = []
    for i, turn in enumerate(session["turns"]):
        if turn["role"] == "user" and not session.get("recorded_only", False):
            for span in sentences(turn["text"]):
                if 8 <= len(span) <= 300:
                    pool.append((i, span))
    signal = [x for x in pool if any(re.search(p, x[1], re.I) for p in SIGNALS)]
    other = [x for x in pool if x not in signal]
    selected = [("signal", x) for x in ranked(signal, session["key"]+":signal")[:5]]
    selected += [("random", x) for x in ranked(other, session["key"]+":random")[:len(selected)]]
    results = []
    for origin, (i, text) in selected:
        kind = "preference" if re.search(r"\bprefer\b", text, re.I) else "decision" if re.search(r"\bwe decided\b", text, re.I) else "constraint"
        results.append({"pool": origin, "stratum": "generated", "text": text, "kind_hint": kind, "turn_index": i,
                        "source_entry_id": session["turns"][i]["entry_id"], "verified_by": None})
    for proposal in session["recorded_proposals"]:
        args = proposal["args"]
        source_id = args.get("source_entry_id", "")
        index = next((i for i, t in enumerate(session["turns"]) if t["entry_id"] == source_id), None)
        results.append({"pool": "recorded", "stratum": "recorded_proposals", "text": args.get("text", ""),
                        "kind_hint": args.get("category", ""), "turn_index": index,
                        "source_entry_id": source_id, "verified_by": args.get("verified_by"),
                        "proposal_turn_index": proposal["turn_index"]})
    for n, result in enumerate(results):
        result.update({"id": digest(encoded([session["key"], n, result])), "session_key": session["key"],
                       "session_id": session["session_id"], "session_timestamp": session["end"], "session_start": session["start"], "project": session["project"]})
    return results


def prepare():
    # Written before accessing session content or computing any labels.
    private_write("protocol.json", RULES)
    private_write("generation-manifest.json", {"protocol": RULES, "generator_matcher_sha256": digest(Path(__file__).read_bytes())})
    scrubber = Scrubber()
    counts, sessions, recorded_contexts, inventory, seen = {}, [], [], [], {}
    roots = {"claude": Path.home()/".claude/projects", "codex": Path.home()/".codex/sessions",
             "icers": Path.home()/".icers", "least": Path("/home/mewtwo/Zykairotis/least")}
    for source, root in roots.items():
        count = collections.Counter()
        for path in allowed_files(root):
            count["files"] += 1
            try:
                raw = path.read_bytes()
            except OSError:
                count["unreadable_files"] += 1
                inventory.append({"source": source, "path_hash": digest(str(path).encode()), "read_error": True})
                continue
            file_id = digest(str(path).encode())
            inventory.append({"source": source, "path_hash": file_id, "sha256": digest(raw), "bytes": len(raw)})
            session, issues = normalize(source, raw, file_id, scrubber)
            count.update(issues)
            count["recorded_proposals_found"] += len(session["recorded_proposals"])
            count["parsed_files"] += 1
            if sum(t["role"] == "user" for t in session["turns"]) < 2:
                count["fewer_than_two_user_turns"] += 1
                if session["recorded_proposals"]:
                    session.update(key=session["content_sha256"], also_sources=[], recorded_only=True)
                    recorded_contexts.append(session)
                    count["recorded_proposals_in_short_sessions"] += len(session["recorded_proposals"])
                continue
            count["eligible_before_dedup"] += 1
            key = session["content_sha256"]
            if key in seen:
                count["duplicates"] += 1
                seen[key]["also_sources"].append(source)
                continue
            session.update(key=key, also_sources=[])
            seen[key] = session
            sessions.append(session)
            count["retained_sessions"] += 1
        counts[source] = dict(count)
    sessions.sort(key=lambda s: (s["start"] if s["start"] is not None else 2**63, s["key"]))
    # The normalized session cohort excludes short sessions, but the separately
    # requested recorded-proposal stratum must replay every recorded call.
    cohort = sorted(sessions + recorded_contexts, key=lambda s: (s["start"] if s["start"] is not None else 2**63, s["key"]))
    candidates = [c for s in cohort for c in generate(s)]
    private_write("sessions.json", cohort)
    private_write("candidates.json", candidates)
    private_write("source-inventory.json", inventory)
    private_write("normalization.json", {"sources": counts, "redactions": dict(scrubber.counts),
                   "date_start": min((s["start"] for s in sessions if s["start"] is not None), default=None),
                   "date_end": max((s["end"] for s in sessions if s["end"] is not None), default=None)})
    print(json.dumps({"sessions": len(sessions), "candidates": len(candidates), "sources": counts}, sort_keys=True))


def label(candidate, sessions):
    at, project = candidate["session_timestamp"], candidate["project"]
    if at is None or project is None:
        return "UNKNOWN", None
    later = [s for s in sessions if s["project"] == project and s["start"] is not None and s["start"] > at]
    for session in later:
        if session["start"] > at + 14*DAY:
            continue
        for i, turn in enumerate(session["turns"]):
            if turn["role"] != "user" or turn["timestamp"] is None or not at < turn["timestamp"] <= at+14*DAY:
                continue
            if any(useful_match(candidate["text"], span) for span in sentences(turn["text"])):
                return "USEFUL", {"session_key": session["key"], "turn_index": i}
    return ("NOT_USEFUL", None) if any(s["start"] >= at+14*DAY for s in later) else ("UNKNOWN", None)


def audit_page(sample, sessions):
    index = {s["key"]: s for s in sessions}
    cards = []
    for n, c in enumerate(sample, 1):
        turns = index[c["session_key"]]["turns"]
        at = c["turn_index"]
        context = "[source span unavailable]" if at is None else "\n\n".join(f'{t["role"]}: {t["text"]}' for t in turns[max(0, at-3):at+4])
        cards.append(f'<section data-id="{c["id"]}"><h2>Candidate {n}</h2><pre>{html.escape(c["text"])}</pre><details><summary>Context ±3 turns</summary><pre>{html.escape(context)}</pre></details><fieldset><legend>Your judgment</legend>' + ''.join(f'<label><input type="radio" name="{c["id"]}" value="{v}">{v}</label> ' for v in ("Useful", "Not useful", "Unsure")) + '</fieldset></section>')
    return ('<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width"><meta http-equiv="Content-Security-Policy" content="default-src \'none\'; style-src \'unsafe-inline\'; script-src \'unsafe-inline\'"><title>Private memory audit</title><style>body{max-width:70ch;margin:2rem auto;padding:1rem;font:18px system-ui}section{border-top:1px solid #888;padding:1rem 0}pre{white-space:pre-wrap;overflow-wrap:anywhere}label{display:inline-block;padding:.6rem}button{padding:.8rem;font:inherit}</style><h1>Private memory audit</h1><p>Local only; no requests, no external assets. Hindsight labels are hidden. Download answers before closing; they are not saved automatically. Keep the downloaded JSON in this heldout directory at mode 600.</p><button id="save">Download answers JSON</button>' + ''.join(cards) + '''<script>
document.getElementById('save').onclick=()=>{const answers=Array.from(document.querySelectorAll('section')).map(s=>({id:s.dataset.id,answer:s.querySelector('input:checked')?.value??null}));const u=URL.createObjectURL(new Blob([JSON.stringify({version:1,answers},null,2)],{type:'application/json'}));const a=document.createElement('a');a.href=u;a.download='audit-answers.json';a.click();setTimeout(()=>URL.revokeObjectURL(u),1000)};
</script></html>''').encode()


def freeze():
    read = lambda n: json.loads((OUT/n).read_bytes())
    if read("protocol.json") != RULES:
        raise RuntimeError("Frozen protocol differs from source; refusing to label")
    sessions, candidates, decisions = read("sessions.json"), read("candidates.json"), read("native-decisions.json")
    decisions = {d["id"]: d for d in decisions}
    if set(decisions) != {c["id"] for c in candidates}:
        raise RuntimeError("Native decisions do not cover the complete candidate denominator")
    # astimezone on the cutoff date uses local historical timezone/DST rules.
    cutoff = int(dt.datetime.fromisoformat(CUTOFF).astimezone().timestamp()*1000)
    summary = {split: {group: collections.Counter({k: 0 for k in ("total", "USEFUL", "NOT_USEFUL", "UNKNOWN", "arm_c_admit", "arm_c_reject")})
                       for group in ("combined", "generated:signal", "generated:random", "recorded_proposals:recorded")}
               for split in ("DEV", "HELD_OUT", "UNKNOWN_TIME")}
    session_counts = collections.Counter("UNKNOWN_TIME" if s["start"] is None else "DEV" if s["start"] < cutoff else "HELD_OUT" for s in sessions)
    for c in candidates:
        c["split"] = "UNKNOWN_TIME" if c["session_start"] is None else "DEV" if c["session_start"] < cutoff else "HELD_OUT"
        c["label"], c["evidence"] = label(c, sessions)
        c["arm_c"] = decisions[c["id"]]
        for group in ("combined", c["stratum"]+":"+c["pool"]):
            counts = summary.setdefault(c["split"], {}).setdefault(group, collections.Counter())
            counts["total"] += 1
            counts[c["label"]] += 1
            counts["arm_c_"+c["arm_c"]["decision"]] += 1
    heldout = [c for c in candidates if c["split"] == "HELD_OUT"]
    sample = []
    for lab in ("USEFUL", "NOT_USEFUL"):
        sample.extend(ranked([c for c in heldout if c["label"] == lab], "audit:"+lab)[:50])
    selected = {c["id"] for c in sample}
    sample.extend(ranked([c for c in heldout if c["id"] not in selected], "audit:fill")[:max(0, 100-len(sample))])
    sample = ranked(sample, "audit:order")
    private_write("labeled-candidates.json", candidates)
    private_write("audit-sample.json", {"ids": [c["id"] for c in sample], "counts": dict(collections.Counter(c["label"] for c in sample))})
    private_write("audit.html", audit_page(sample, sessions), raw=True)
    import subprocess
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    sources = ["scripts/prepare_cognee_heldout.py", "crates/ice-cli/src/cognee/heldout_tests.rs"]
    sources += [f"crates/ice-cli/src/cognee/{n}.rs" for n in ("candidates", "dream", "propose", "source_validation", "validation", "text", "evidence")]
    data_hashes = {}
    for path in sorted(OUT.rglob("*")):
        if path.is_symlink():
            raise RuntimeError("Private artifact symlink violation")
        if path.is_dir():
            if stat.S_IMODE(path.stat().st_mode) != 0o700:
                raise RuntimeError("Private directory permission violation")
            continue
        if not path.is_file() or stat.S_IMODE(path.stat().st_mode) != 0o600:
            raise RuntimeError("Private artifact permission/type violation")
        data_hashes[str(path.relative_to(OUT))] = digest(path.read_bytes())
    normalization = read("normalization.json")
    manifest = {"schema_version": 1, "git_head": head, "protocol": RULES,
                "cutoff_epoch_ms": cutoff, "cutoff_with_offset": dt.datetime.fromisoformat(CUTOFF).astimezone().isoformat(),
                "source_hashes": {n: digest((ROOT/n).read_bytes()) for n in sources},
                "data_sha256": data_hashes, "counts": summary, "session_counts": dict(session_counts), "normalization": normalization,
                "audit_count": len(sample), "audit_label_counts": dict(collections.Counter(c["label"] for c in sample)),
                "heldout_known_labels": sum(c["label"] != "UNKNOWN" for c in heldout),
                "remote_requests": 0, "arm_d_executed": False}
    private_write("manifest.json", manifest)
    # Public copy deliberately contains aggregates, fixed rules and hashes, never session fields.
    print(json.dumps({"manifest_sha256": digest(encoded(manifest)), "counts": summary,
                      "normalization": normalization, "audit_count": len(sample),
                      "heldout_known_labels": manifest["heldout_known_labels"]}, sort_keys=True))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("phase", choices=("prepare", "freeze"))
    args = parser.parse_args()
    os.umask(0o077)
    (prepare if args.phase == "prepare" else freeze)()


if __name__ == "__main__":
    main()
