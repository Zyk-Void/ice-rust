"""Build the complete audit index and typed dependency order from accepted evidence only."""
import csv, hashlib, json, re, subprocess, sys
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path
sys.setrecursionlimit(10000)
B=Path(__file__).resolve().parent.parent
E=B/"evidence"
AREAS=["ai","agent","tools","session","settings","extensions","rpc-remote","tui","subagents","safe-verify","observability","cognee","blackhole","providers-auth","cli-packaging","misc"]
mode=sys.argv[1] if len(sys.argv)>1 else "draft"
if mode not in ("draft","complete"): raise ValueError("draft or complete")
def load(n): return json.loads((E/n).read_text())
def write(n,v): (E/n).write_text(json.dumps(v,indent=2,ensure_ascii=False)+"\n")
def hashfile(p): return hashlib.sha256(p.read_bytes()).hexdigest()
def table(p):
    with p.open(newline="") as f: return list(csv.DictReader(f,strict=True))
def md(v): return str(v).replace("|","&#124;").replace("\n"," ").strip()
def human(v):
    if isinstance(v,str): return v
    if isinstance(v,dict):
        useful=[str(v[k]) for k in ("question","reason","capability","ts_comparison","finding","limitation","behavior","note","description") if v.get(k)]
        return " ".join(useful) if useful else json.dumps(v,ensure_ascii=False)
    return str(v)
specs={a:load("spec-"+a+".json") for a in AREAS}
manifests={a:load(a+".json") for a in AREAS}
entries=[dict(e,area=a) for a,s in specs.items() for e in s["entries"]]
byid={e["id"]:e for e in entries}
if len(byid)!=len(entries): raise ValueError("Duplicate IDs")
progress=load("progress.json")
if progress.get("complete") and mode!="complete": raise ValueError("Published audit is protected")
if mode=="complete":
    gate=load("final-validation.json")
    if not gate.get("passed") or gate.get("phase")!="active": raise ValueError("Active final gate must pass first")
    if Path(gate["identities"]["rust"]["root"]).resolve()!=B.parents[1]: raise ValueError("Gate from wrong checkout")
edges=[dict(from_id=e["id"],to=r["entry_id"],kind=r["kind"],reason=r.get("reason",""),reviews=r.get("reviews",[])) for e in entries for r in e.get("dependency_relations",[])]
for e in edges:
    if e["to"] not in byid: raise ValueError("Unknown dependency")
adj={e["id"]:sorted(e["dependencies"]) for e in entries}
number=0; stack=[]; onstack=set(); index={}; low={}; components=[]
def visit(v):
    global number
    index[v]=low[v]=number;number+=1;stack.append(v);onstack.add(v)
    for w in adj[v]:
        if w not in index: visit(w);low[v]=min(low[v],low[w])
        elif w in onstack: low[v]=min(low[v],index[w])
    if low[v]==index[v]:
        c=[]
        while True:
            w=stack.pop();onstack.remove(w);c.append(w)
            if w==v: break
        components.append(sorted(c))
for v in sorted(byid):
    if v not in index: visit(v)
components.sort(key=lambda c:c[0])
component={v:i for i,c in enumerate(components) for v in c}
needs={i:{component[w] for v in c for w in adj[v] if component[w]!=i} for i,c in enumerate(components)}
dependents=defaultdict(set)
for i,ds in needs.items():
    for d in ds: dependents[d].add(i)
def rank(i): return (min(byid[v]["priority"] for v in components[i]),min(AREAS.index(byid[v]["area"]) for v in components[i]),components[i][0])
ready=sorted([i for i,d in needs.items() if not d],key=rank); ordered=[]
while ready:
    i=ready.pop(0);ordered.append(i)
    for n in dependents[i]:
        needs[n].remove(i)
        if not needs[n]: ready.append(n)
    ready.sort(key=rank)
if len(ordered)!=len(components): raise ValueError("Invalid condensed dependency DAG")
groups=[]; order=[]
for position,i in enumerate(ordered,1):
    name=f"G{position:03d}"
    members=sorted(components[i],key=lambda v:(byid[v]["priority"],AREAS.index(byid[v]["area"]),v))
    groups.append({"group":name,"entry_ids":members,"mutually_dependent":len(members)>1,"entry_count":len(members)})
    for v in members:
        e=byid[v]
        order.append({"order":len(order)+1,"group":name,"entry_id":v,"area":e["area"],"priority":e["priority"],"prerequisites":";".join(e["dependencies"]),"related_entries":";".join(r["entry_id"]+":"+r["kind"] for r in e.get("dependency_relations",[]) if r["kind"]!="prerequisite")})
graph={"schema_version":1,"captured_at":datetime.now(timezone.utc).isoformat(),"edge_direction":"from entry requires/references to entry; only prerequisite edges constrain order","nodes":[{k:e[k] for k in ("id","area","title","status","priority","size_min","size_max")} for e in entries],"edges":[{"from":e["from_id"],**{k:v for k,v in e.items() if k!="from_id"}} for e in edges],"groups":groups,"method":"Tarjan strongly connected components followed by dependency-first topological order; priority and stable area/ID break ties. Cyclic prerequisite contracts stay in one explicit implementation group rather than deleting an edge."}
write("dependency-graph.json",graph)
with (E/"porting-order.csv").open("w",newline="") as f:
    w=csv.DictWriter(f,fieldnames=["order","group","entry_id","area","priority","prerequisites","related_entries"]);w.writeheader();w.writerows(order)
ledger=table(B/"ledger.csv")
surfaces={p.stem:table(p) for p in sorted((B/"surfaces").glob("*.csv"))}
fpc=load("qa-first-pass-coverage.json");defects=load("rust-defects.json");exports=load("qa-unnamed-exports-dispositions.json")
strengths=[];questions=[]
for area,m in manifests.items():
    for r in m.get("rust_only",[]): strengths.append({"area":area,**(r if isinstance(r,dict) else {"capability":r})})
    for key in ("open_questions","unverified"):
        value=m.get(key,[])
        records=value if isinstance(value,list) else [value]
        for r in records: questions.append({"area":area,"kind":key,"record":r,"text":human(r)})
write("native-strengths.json",{"records":strengths,"source":"Accepted area manifests; no new capability inference"})
unique={q["area"]+"|"+q["kind"]+"|"+q["text"]:q for q in questions}
questions=list(unique.values())
write("open-questions.json",{"source_only":True,"records":questions,"external_dependency_limits":load("dependency-final-reconciliation.json").get("external_unverified",[])})
actual={"ledger_rows":len(ledger),"mapped":sum(r["status"]=="mapped" for r in ledger),"not_applicable":sum(r["status"]=="not-applicable" for r in ledger),"pending":sum(r["status"]=="pending" for r in ledger),"entry_count":len(entries),"first_pass_claims":sum(len(m.get("claims",[])) for m in manifests.values()),"surface_counts":{a:{"total":len(rr),"pending":sum(r["status"]=="pending" for r in rr)} for a,rr in surfaces.items()}}
progress.update(actual);progress["captured_at"]=datetime.now(timezone.utc).isoformat();progress["complete"]=mode=="complete"
progress["areas"]=[{"area":a,"complete":mode=="complete","entries":len(specs[a]["entries"]),"published":True} for a in AREAS]
progress["first_pass_matrix_claims"]=len(fpc["claims"]);progress["unnamed_export_review"]=exports["summary"]
progress["phase"]="complete-active" if mode=="complete" else "final-validation"
write("progress.json",progress)
baseline=load("baseline.json")
latest_integrity=sorted(E.glob("resume-integrity-*.json"))[-1].name
active=B.parents[1].as_posix().startswith("/home/mewtwo/Zykairotis/")
native=load("active-checkout-source-map.json")["identities"]["active"] if active else baseline["rust"]
status="COMPLETE — active checkout verified" if mode=="complete" else ("Original checkout validated; active relocation pending" if not active and (E/"final-metadata-original.json").exists() else "All source contracts published; final validation in progress")
out=["# ICE → ICE-Rust parity and porting specification","",status+".","",
"This is a source-referenced audit and implementation specification. Each entry preserves the TypeScript behavior, its exact interfaces, the native counterpart, and the required difference. Evidence limitations are stated separately from missing features. Tests were read where useful; no application, test suite, build, updater, service or private user-state operation was run.","",
"## Frozen sources and audit scope","",
"| Source | Root | Branch | Commit |","|---|---|---|---|",
f"| TypeScript | {md(baseline['ts']['root'])} | {md(baseline['ts']['branch'])} | {baseline['ts']['head']} |",
f"| Native comparison | {md(native['root'])} | {md(native['branch'])} | {native['head']} |","",
f"The TypeScript dirty baseline is intentional and retained in [baseline.json](evidence/baseline.json); [source integrity](evidence/{latest_integrity}) records the resume comparison. The original Rust baseline is retained unchanged for provenance. Generated model data is inventoried under the stated method exception; its wrapper behavior is reviewed without claiming every generated model row was read.",""]
if active: out += ["The native runtime now uses the icers binary, .icers paths and ICERS_* environment names. TypeScript continues to use ice, .ice and ICE_*. Crate names remain ice-* and the plugin registration symbol remains ice_plugin_register. [Relocation evidence](evidence/relocation-rust-remap.json) gives each native citation's old range, new range and review method.",""]
out+=["## Final self-check","",f"The specification contains **{len(entries)} entries in 16 areas**. The file ledger contains **{len(ledger)} rows: {actual['mapped']} mapped, {actual['not_applicable']} justified not applicable, {actual['pending']} pending**. The surface inventory contains **{sum(len(v) for v in surfaces.values())} rows**, with **{sum(x['pending'] for x in actual['surface_counts'].values())} pending**. Counts are computed from the actual CSV and accepted JSON files.","","| Area | Entries | Missing | Partial | Different by design | P0 | P1 |","|---|---:|---:|---:|---:|---:|---:|"]
for a,s in specs.items():
    cc=Counter(e["status"] for e in s["entries"]);pp=Counter(e["priority"] for e in s["entries"])
    out.append(f"| [{a}]({a}.md) | {len(s['entries'])} | {cc['missing']} | {cc['partial']} | {cc['different-by-design']} | {pp['P0']} | {pp['P1']} |")
out+=["","The 151 TUI entries already include the 67 terminal entries TUI-201–267. They are counted once. AI-019 is integrated; SESSION-039 and SESSION-040 remain unused because the reviewed context/cache features have native counterparts.","","| Surface | Rows | Pending |","|---|---:|---:|"]
for a,rr in surfaces.items():out.append(f"| [{a}](surfaces/{a}.csv) | {len(rr)} | {actual['surface_counts'][a]['pending']} |")
out+=["",f"Export reconciliation covers **{exports['summary']['records']} records: {exports['summary']['runtime']} runtime and {exports['summary']['type']} type exports; {exports['summary']['unresolved']} unresolved**. Every record is named in an owning contract or has a source-reviewed disposition, including existing native counterparts, aliases and test support. [Full export dispositions](evidence/qa-unnamed-exports-dispositions.json).","",
"All 791 How-it-works sections pass the behavioral-opening check. The central spacing pass changed 5,359 delimiters/word-number boundaries after reviewing 110 distributed examples, with zero changes to extracted citations, identifiers or original protected spans. [Spacing proof](evidence/spacing-validation.json). The residual pass reviewed every proposed edit and corrected remaining ordinary-prose glue, including numeric labels. Its [complete change list](evidence/spacing-residual-dry-run.json) and [validation proof](evidence/spacing-residual-validation.json) preserve byte-identical citations, identifiers and protected spans. The final gate also reruns the read-only residual check against the current rendered audit and index.","",
"## Dependency graph and suggested porting order","",
f"The [complete typed dependency graph](evidence/dependency-graph.json) contains {len(entries)} nodes and {len(edges)} directed edges: "+", ".join(f"{n} {k}" for k,n in Counter(e['kind'] for e in edges).items())+". Arrows run from a contract to the contract it requires or references. Consumers and shared contracts are retained separately; they do not constrain prerequisite ordering.","",
f"The [complete porting order](evidence/porting-order.csv) lists every entry once in {len(groups)} dependency groups. "+str(sum(g['mutually_dependent'] for g in groups))+" groups contain mutually dependent contracts and must be planned together. Within an available group, priority precedes stable area/ID order. Estimated lines are planning estimates per entry, not measured work or safely additive commitments.","",
"### Cross-area prerequisite graph","",
"This adjacency table is the area projection of every prerequisite edge. The JSON retains exact entry identities, edge kinds, reasons and review IDs.","",
"| Area | Required areas (edge count) |","|---|---|"]
for a in AREAS:
    cc=Counter(byid[e["to"]]["area"] for e in edges if e["kind"]=="prerequisite" and byid[e["from_id"]]["area"]==a and byid[e["to"]]["area"]!=a)
    out.append("| "+a+" | "+(", ".join(f"{x} ({n})" for x,n in sorted(cc.items())) or "none")+" |")
out+=["","The optional Goals scheduler's full state producer remains external and unverified. OBS-009 keeps its verified SAFE host contracts; no fictitious MISC producer is assigned. [Dependency reconciliation](evidence/dependency-final-reconciliation.json).","",
"### First available P0/P1 contracts","",
"| Order | Group | Entry | Contract | Priority |","|---:|---|---|---|---|"]
top=[r for r in order if r["priority"] in ("P0","P1")][:10]
for r in top:
    e=byid[r["entry_id"]];out.append(f"| {r['order']} | {r['group']} | {e['id']} | {md(e['title'])} | {e['priority']} |")
out+=["","## Rust defects, provenance and native strengths","",
f"The separate [native defect catalog](evidence/rust-defects.json) contains **{len(defects)} source-level records**. It distinguishes implementation defects and conditional risks from missing TypeScript features. These findings were not reproduced by executing the application.","",
"| Severity | Records |","|---|---:|"]
for k,n in sorted(Counter(re.split(r"[ ;(]",d["severity"],maxsplit=1)[0] for d in defects).items()):out.append(f"| {md(k)} | {n} |")
out+=["","| Provenance | Records |","|---|---:|"]
for k,n in sorted(Counter(d.get("origin_provenance_status",d.get("provenance_status","documented per record")) for d in defects).items()):out.append(f"| {md(k)} | {n} |")
out+=["","RUST-066 (visual-only new/clear command), RUST-067 (blocking share child process), and RUST-068 (queue failure does not restore the prompt draft) are traced to the inspected upstream ancestry. [Local Git evidence](evidence/rust-defect-provenance.json) and [coordinator decisions](evidence/dependency-coordinator-decisions.json) preserve the exact targets and limitations.",""]
if active and next((d for d in defects if d["id"]=="RUST-045"),{}).get("active_status")=="fixed":out+=["RUST-045 is fixed in the active tree: the repository constants are canonical lowercase in rust-active:crates/ice-cli/src/prebuilt_release.rs:11 and rust-active:crates/ice-cli/src/changelog.rs:27. The original finding remains in the provenance catalog with its active status. Severity totals include this historical high-severity record; the other 69 records retain their individual source-level conditions and evidence limits.",""]
if active and any(d["id"]=="RUST-069" for d in defects):out+=["RUST-069 records a conditional native compatibility regression introduced by the local icers identity change. If historical ice.value custom records reach SessionValues without an external migration, its new exact icers.value filter omits them from the restored value view. The generic log records can remain intact. A concrete caller restores display.hide_thinking through that API; no real user session or runtime reproduction was inspected (rust:crates/ice-harness/src/session/values.rs:31-82; rust:crates/ice-cli/src/interactive_tui.rs:917-928). The catalog preserves the introducing commit, native caller/decode/query trace and explicit external-migration limitation.",""]
out+=["### Native capabilities to preserve","",
"Native implementations are credited within each comparison. Keep transactional plugin installation and artifact ownership, prebuilt extension verification, update caching and rollback, durable session storage, native event/provider/plugin integration, deterministic harness tests, and existing terminal interaction when adding parity. The accepted [native strengths inventory](evidence/native-strengths.json) retains all area-provided capabilities and source references.","",
"| Area | Existing capability |","|---|---|"]
for a in AREAS:
    rr=[r for r in strengths if r["area"]==a]
    if rr:
        r=rr[0];cap=human(r);ev=r.get("evidence",[])
        if isinstance(ev,str):ev=[ev]
        out.append(f"| {a} | {md(cap)} "+md("; ".join(ev))+" |")
out+=["","## First-pass report reconciliation","",
f"All **{len(fpc['claims'])} claims** in the [first-pass matrix](evidence/qa-first-pass-coverage.json) have a source-based disposition, including §11a's four active-import topics. There are no uncovered claims or draft-dependent follow-ups. The accepted area manifests contain {actual['first_pass_claims']} claim records; that different count is not a missing-coverage signal because the normalized matrix splits and deduplicates report claims.","",
"| Disposition | Claims |","|---|---:|"]
for k,n in sorted(Counter(c["disposition"] for c in fpc["claims"]).items()):out.append(f"| {k} | {n} |")
out+=["","### Corrections to the first-pass report","",
"| Claim | Original claim | Source-based correction | Entries |","|---|---|---|"]
for c in fpc["claims"]:
    if c["disposition"]=="corrected":out.append(f"| {c['id']} | {md(c['claim'])} | {md(c['finding'])} | {', '.join(c.get('entry_ids',[]))} |")
out+=["","Private import contents and outcomes were not read or inferred. Source comparison resolves the public format questions: unknown thinking-level keys, compiled versus configurable provider catalogs, typed native themes versus TS JSON palettes, and actual settings schema membership. The matrix preserves the limits on the private historical claims.","",
"## Open questions and evidence limits","",
"Implementation choices that source alone cannot settle remain explicit. They do not reopen the completed inventory:","",
"- Decide compatibility policy for deliberately different native runtime identities, persisted formats and malformed-input defenses.",
"- Decide whether visual-only /clear remains separate from true /new semantics; RUST-066 records the current behavior and user-visible label ambiguity.",
"- Confirm external package, Goals producer, bridge, release-service and deployment behavior independently if implementation relies on it; none was contacted.",
"- Private credentials, private migration outcomes and real ~/.ice or ~/.icers contents remain outside the source audit.",
"- The canonical coverage inventory retains 1,702 paths, including 26 byte-identical copies whose coverage is transparently derived from reviewed originals. It excludes 86 connector-redacted lines and the single historical sessions.rs:627 evidence conflict from visible read credit.",
"- Five initially dirty TypeScript test files lacked initial worktree hashes: bash-watch.test.ts, extensions-input-event.test.ts, status-line.test.ts, execution-reliability-1-watch-alerts.test.ts and packages/tui/test/editor.test.ts. Current read hashes are verified; they do not prove those files matched an uncaptured historical state.",
"- Generated image-model values at image-models.generated.ts:36–819 are grouped under requirements.md line 23; only lines 1–35 plus the complete generator and consumer are source-read. Remaining partial supporting-file ranges stay explicit in the canonical coverage QA.",
"- Terminal empty split-line endpoints are accepted as file-ending citation bounds and reported as warnings, never counted as additional source behavior. [Coverage reconciliation QA](evidence/lane-final-areas-read-coverage-qa.json) retains the exact exclusions and partial supporting ranges.",
"",
f"The [complete area question and limitation inventory](evidence/open-questions.json) contains {len(questions)} retained records. [Canonical read coverage](evidence/read-coverage.json) records exact ranges and exclusions. [Final validation](evidence/final-validation.json) records source identities, drift checks, structural/citation results, and validation limits.","",
"## All-entry index","",
"| ID | Title | Status | Priority | Estimated Rust lines |","|---|---|---|---|---:|"]
for e in entries:
    slug=re.sub(r"[^\w\s-]","",e["id"]+" - "+e["title"]).lower().replace(" ","-")
    out.append(f"| [{e['id']}]({e['area']}.md#{slug}) | {md(e['title'])} | {e['status']} | {e['priority']} | {e['size_min']}–{e['size_max']} |")
out+=["","## Evidence and tools","",
"The authoritative method is [requirements.md](requirements.md). The latest work record is [RESUME.md](evidence/RESUME.md); historical evidence and lane proposals remain under evidence/. Documentation helpers live under tools/ in the relocated result. Re-run the source-only final gate from outside this directory using Python isolated mode:","",
chr(96)*3+"sh", "/usr/bin/python3 -I "+str(B/"tools/verify-complete-audit.py")+(" active" if active else " original"),chr(96)*3,""]
if active: out += ["Unfinished independent-review material is preserved without editing its findings under [evidence/independent-review/](evidence/independent-review/). It remains a historical reviewer archive, not a completed audit deliverable.",""]
if (E/"lane-native-final-spacing-preview-incident.json").exists(): out += ["A legacy helper import overwrote the first-pass dry-run preview during closeout preparation. The original validation and approval remain intact, including all 5,359 accepted changes and the byte-preservation proof; the preview's original bytes could not be recovered. The [incident record](evidence/lane-native-final-spacing-preview-incident.json) preserves the expected and replacement hashes. The residual pass has its own complete dry-run and validation artifacts.",""]
if (E/"active-closeout-corrections.json").exists(): out += ["Final closeout checks found and corrected an index-helper invocation error and one RUST-069 citation path. The [closeout correction record](evidence/active-closeout-corrections.json) preserves the repeated preview-write history, pre-correction artifacts, the failed active gate, and exact helper/citation amendments. Original frozen migration and acceptance evidence remains unchanged.",""]
readme_text="\n".join(out)
normalizer="import fs from 'node:fs';import {pathToFileURL} from 'node:url';const m=await import(pathToFileURL(process.argv[2]).href);process.stdout.write(m.normalizeResidualSpacing(fs.readFileSync(0,'utf8')));"
readme_text=subprocess.run(["node","--input-type=module","-e",normalizer,"ice-gap-readme-normalizer",str(B/"tools/fix-spacing.mjs")],input=readme_text,text=True,capture_output=True,check=True).stdout
if not readme_text.startswith("# "): raise ValueError("README renderer emitted unexpected output before its heading")
(B/"README.md").write_text(readme_text)
write("index-generation.json",{"captured_at":progress["captured_at"],"source_only":True,"mode":mode,"entries":len(entries),"groups":len(groups),"cyclic_groups":[{"group":g["group"],"size":g["entry_count"]} for g in groups if g["mutually_dependent"]],"top_priority_order":top,"native_strengths":len(strengths),"question_records":len(questions),"inputs":[{"area":a,"sha256":hashfile(E/("spec-"+a+".json"))} for a in AREAS]})
print(json.dumps({"entries":len(entries),"ledger":actual,"edges":len(edges),"groups":len(groups),"cyclic_groups":[{"group":g["group"],"size":g["entry_count"]} for g in groups if g["mutually_dependent"]],"top_priority_order":top,"native_strengths":len(strengths),"questions":len(questions),"readme_bytes":(B/"README.md").stat().st_size},indent=2))
