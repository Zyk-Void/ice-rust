#!/usr/bin/env python3
"""Prepare private owner-labeling input; never calls a model or Cognee.

Run with python3 -I. Existing private outputs are never overwritten.
"""
import collections
import hashlib
import json
import os
from pathlib import Path
import re
import runpy
import subprocess

ROOT = Path(__file__).resolve().parents[1]
BASE = runpy.run_path(str(ROOT / "scripts/prepare_cognee_heldout.py"))
OUT = ROOT / "target/judged-eval"
SEED = 20261028


def hashed(data):
    return hashlib.sha256(data).hexdigest()


def private_write(name, data):
    OUT.mkdir(mode=0o700, parents=True, exist_ok=True)
    if OUT.is_symlink():
        raise ValueError("private output directory must not be a symlink")
    os.chmod(OUT, 0o700)
    fd = os.open(OUT / name, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "wb") as stream:
        stream.write(data)


def bounded(text, limit):
    return text.encode("utf-16-le")[:limit * 2].decode("utf-16-le", errors="ignore")


def sample():
    # Same source allowlist as heldout preparation, except live ~/.icers is
    # explicitly forbidden for this task. No fallback into .ice or a store.
    roots = {"claude": Path.home() / ".claude/projects",
             "codex": Path.home() / ".codex/sessions",
             "least": Path("/home/mewtwo/Zykairotis/least")}
    scrubber = BASE["Scrubber"]()
    buckets = collections.defaultdict(list)
    session_counts = collections.Counter()
    for source, root in roots.items():
        if root.is_symlink():
            raise ValueError("source root must not be a symlink")
        for path in BASE["allowed_files"](root):
            # Reuse both the parsers and the mixed tool-envelope filtering.
            session, _ = BASE["normalize"](source, path.read_bytes(), hashed(str(path).encode()), scrubber)
            session_key = hashed((source + ":" + session["session_id"]).encode())
            previous = ""
            eligible = []
            for index, turn in enumerate(session["turns"]):
                if turn["role"] == "assistant" and turn["text"].strip():
                    previous = bounded(turn["text"], 800)
                if turn["role"] != "user":
                    continue
                prompt = turn["text"].strip()
                if not prompt or re.fullmatch(r"/[^\s]+(?:\s*)", prompt):
                    continue
                if len(prompt.encode("utf-16-le")) > 16000:
                    continue  # Do not silently change the owner's actual prompt.
                at = turn["timestamp"] or session["start"]
                if at is None:
                    continue
                row = {"id": hashed(f"{session_key}:{index}".encode()),
                       "source": source, "session": session_key, "timestamp": at,
                       "prompt": prompt, "previous_assistant": previous}
                eligible.append(row)
            eligible.sort(key=lambda row: hashed(f"{SEED}:{row['id']}".encode()))
            for row in eligible:
                if session_counts[session_key] >= 3:
                    break
                session_counts[session_key] += 1
                buckets[(source, row["timestamp"] // 86400000)].append(row)
    # Round-robin source/day strata, then seed-ranked prompts within each.
    for rows in buckets.values():
        rows.sort(key=lambda row: hashed(f"{SEED}:within:{row['id']}".encode()))
    keys = sorted(buckets, key=lambda key: hashed(f"{SEED}:stratum:{key}".encode()))
    selected = []
    while keys and len(selected) < 200:
        for key in keys[:]:
            selected.append(buckets[key].pop())
            if not buckets[key]:
                keys.remove(key)
            if len(selected) == 200:
                break
    selected.sort(key=lambda row: hashed(f"{SEED}:page:{row['id']}".encode()))
    return selected


PAGE = '''<!doctype html>
<html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<title>Memory Recall Labels</title>
<style>
/* One private prompt at a time, with the prior turn as supporting evidence. */
:root { --bg:#edf2f6; --fg:#182e40; --paper:#fff; --muted:#4c6474; --accent:#126a87; --line:#9eafbc; --display:Georgia,serif; --body:system-ui,sans-serif; }
@media(prefers-color-scheme:dark){:root:not([data-theme="light"]){--bg:#152531;--fg:#e8f1f6;--paper:#203746;--muted:#b4c8d5;--accent:#77d4ec;--line:#597283;color-scheme:dark}}
:root[data-theme="dark"]{--bg:#152531;--fg:#e8f1f6;--paper:#203746;--muted:#b4c8d5;--accent:#77d4ec;--line:#597283;color-scheme:dark}
body{margin:0;background:var(--bg);color:var(--fg);font:16px/1.6 var(--body);padding-block:24px;padding-left:max(16px,env(safe-area-inset-left,0px));padding-right:max(16px,env(safe-area-inset-right,0px))}
main{max-width:760px;margin:auto;display:grid;gap:18px;min-width:0}h1{font:2rem var(--display);margin:0}h2{font-size:1rem;color:var(--muted)}pre{white-space:pre-wrap;overflow-wrap:anywhere;font:inherit;margin:0}article{background:var(--paper);padding:20px;border:1px solid var(--line)}nav{display:flex;flex-wrap:wrap;gap:10px}button{font:inherit;border:1px solid var(--accent);color:var(--fg);background:var(--paper);padding:10px 16px;cursor:pointer}button:focus-visible{outline:3px solid var(--accent);outline-offset:3px}button[aria-pressed="true"]{border-width:3px}#progress,#notice{color:var(--muted)}
</style><main><h1>Does this need older memory?</h1><p>Label the information needed to act. “This session” means the current conversation already contains it. “Older memory” means earlier sessions, past decisions or project history. No model is called.</p><p id="progress" aria-live="polite"></p><article><h2>Previous assistant turn · excerpt</h2><pre id="previous"></pre><h2>User prompt</h2><pre id="prompt"></pre></article><nav id="choices" aria-label="Owner label"></nav><nav><button id="back">Previous</button><button id="next">Next</button><button id="download">Download owner-labels.json</button></nav><p id="notice">Labels are stored only in this browser. Download a backup before closing.</p></main>
<script id="sample" type="application/json">SAMPLE_JSON</script>
<script>
'use strict';
const sample=JSON.parse(document.getElementById('sample').textContent), key='judged-labels-SAMPLE_HASH';
let labels={}, index=0;
try {const saved=JSON.parse(localStorage.getItem(key)||'{}');labels=saved.labels||{};index=Math.max(0,Math.min(sample.length-1,saved.index||0));} catch(_) {}
const options=[['none','None'],['session','This session'],['memory','Older memory'],['unsure','Unsure']];
function save(){try{localStorage.setItem(key,JSON.stringify({labels,index}));}catch(_){document.getElementById('notice').textContent='Browser storage is unavailable. Download labels to keep them.';}}
function render(){const row=sample[index];document.getElementById('progress').textContent=`${index+1} / ${sample.length} · ${Object.keys(labels).length} labeled`;
document.getElementById('previous').textContent=row.previous_assistant||'(No previous assistant text)';document.getElementById('prompt').textContent=row.prompt;
const choices=document.getElementById('choices');choices.replaceChildren();for(const [value,title] of options){const b=document.createElement('button');b.id='label-'+value;b.textContent=title;b.setAttribute('aria-pressed',String(labels[row.id]===value));b.onclick=()=>{labels[row.id]=value;if(index<sample.length-1)index++;save();render();};choices.append(b);}}
document.getElementById('back').onclick=()=>{index=Math.max(0,index-1);save();render();};document.getElementById('next').onclick=()=>{index=Math.min(sample.length-1,index+1);save();render();};
document.getElementById('download').onclick=()=>{const blob=new Blob([JSON.stringify({sample_sha256:'SAMPLE_HASH',labels},null,2)],{type:'application/json'});const url=URL.createObjectURL(blob);const a=document.createElement('a');a.href=url;a.download='owner-labels.json';a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);};render();
</script></html>
'''


def main():
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--freeze-existing", action="store_true", help="freeze hashes of already prepared private files without resampling")
    args = parser.parse_args()
    if args.freeze_existing:
        data = (OUT / "sample.json").read_bytes()
        rows = json.loads(data)
        page = (OUT / "label.html").read_bytes()
        sample_hash = hashed(data)
    else:
        rows = sample()
        if not rows:
            raise SystemExit("No eligible prompts in allowed non-live sources")
        data = BASE["encoded"](rows)
        sample_hash = hashed(data)
        page = PAGE.replace("SAMPLE_JSON", json.dumps(rows, ensure_ascii=False).replace("<", "\\u003c")).replace("SAMPLE_HASH", sample_hash).encode()
        private_write("sample.json", data)
        private_write("label.html", page)
        counts = dict(collections.Counter(row["source"] for row in rows))
        private_write("preparation.json", BASE["encoded"]({"counts": counts, "total": len(rows), "seed": SEED, "excluded_source": "live icers state is forbidden"}))
    counts = dict(collections.Counter(row["source"] for row in rows))
    manifest = {"head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
                "sample_sha256": sample_hash, "page_sha256": hashed(page), "seed": SEED,
                "runner_sha256": hashed((ROOT / "crates/ice-cli/src/cognee/judged_eval_tests.rs").read_bytes()),
                "prompt_builder_sha256": hashed((ROOT / "crates/ice-cli/src/cognee/helper.rs").read_bytes()),
                "preparation_sha256": hashed(Path(__file__).read_bytes())}
    path = ROOT / "Docs/cognee-jev-memory/evidence/judged-eval-manifest.json"
    # Manifest is hash-only: no prompt text, identifiers, labels or source paths.
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x") as stream:
        json.dump(manifest, stream, indent=2)
        stream.write("\n")
    print(json.dumps({"total": len(rows), "by_source": counts, "page": str(OUT / "label.html")}))


if __name__ == "__main__":
    main()
