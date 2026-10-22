"""Resume integrity checks; documentation evidence only, no application execution."""
import csv, hashlib, json, os, re, subprocess
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path

HERE=Path(__file__).resolve().parent
BASE=HERE.parent if HERE.name=="tools" else HERE
ROOTS={"ice":Path("/home/mewtwo/Zks/ice"),"rust":BASE.parents[1]}
E=BASE/"evidence"
STAMP=datetime.now(timezone.utc).strftime("%Y-%m-%dT%H%M%SZ")
def load(p): return json.loads(p.read_text())
def digest(p): return hashlib.sha256(p.read_bytes()).hexdigest()
def dump(p,value):
    if not p.resolve().is_relative_to(BASE.resolve()): raise RuntimeError("write boundary")
    p.write_text(json.dumps(value,indent=2,ensure_ascii=False)+"\n")
def csv_rows(p,columns):
    with p.open(newline="") as f:
        reader=csv.DictReader(f,strict=True)
        if reader.fieldnames!=columns: raise ValueError(f"CSV columns: {p}: {reader.fieldnames}")
        out=list(reader)
    if any(None in r or any(v is None for v in r.values()) for r in out): raise ValueError(f"CSV row: {p}")
    return out
def source(side,rel):
    p=(ROOTS[side]/rel).resolve()
    if not p.is_relative_to(ROOTS[side].resolve()): raise ValueError("source boundary")
    return p
def git(root,*args):
    return subprocess.check_output(["git","--no-optional-locks","-C",str(root),*args],text=True)
baseline=load(E/"baseline.json")
progress=load(E/"progress.json")
tracked=load(E/"tracked-files.json")
read_coverage=load(E/"read-coverage.json")
ledger=csv_rows(BASE/"ledger.csv",["path","lines","area","entry_ids","status","notes"])
surface_data={p.stem:csv_rows(p,["name","ice_path:line","rust_path:line_or_blank","status","entry_id"]) for p in sorted((BASE/"surfaces").glob("*.csv"))}
area_order=["safe-verify","cognee","blackhole","agent","extensions","tools","session","subagents","observability","settings","ai","providers-auth","rpc-remote","cli-packaging","misc","tui"]
areas=[a for a in area_order if (E/f"{a}.json").exists()]
specs={a:load(E/f"spec-{a}.json") for a in areas}
manifests={a:load(E/f"{a}.json") for a in areas}
issues=[]
all_entries=[e for s in specs.values() for e in s["entries"]]
id_counts=Counter(e["id"] for e in all_entries)
ids=set(id_counts)
draft_ids={e["id"] for a in area_order if (E/f"spec-{a}.json").exists() for e in load(E/f"spec-{a}.json")["entries"]}
pending_cross_area_references=[]
issues.extend({"kind":"duplicate_id","id":k,"count":v} for k,v in id_counts.items() if v!=1)
for area,spec in specs.items():
    md=(BASE/f"{area}.md").read_text()
    if re.findall(r"^## ([A-Z]+-\d+) - ",md,re.M)!=[e["id"] for e in spec["entries"]]: issues.append({"kind":"markdown_entry_order","area":area})
    for e in spec["entries"]:
        for k in ["id","title","status","source","what","mechanism","interfaces","rust","port","priority","priority_reason"]:
            if not isinstance(e.get(k),str) or not e[k].strip(): issues.append({"kind":"missing_field","id":e["id"],"field":k})
        if not isinstance(e.get("dependencies",[]),list) or not isinstance(e.get("size_min"),(int,float)) or not isinstance(e.get("size_max"),(int,float)): issues.append({"kind":"schema","id":e["id"]})
        if e["status"] not in ["missing","partial","different-by-design"]: issues.append({"kind":"status","id":e["id"]})
    if len(re.findall(r"^### [1-9]\. ",md,re.M))!=9*len(spec["entries"]): issues.append({"kind":"nine_markdown_fields","area":area})
    if [e["id"] for e in manifests[area]["entries"]]!=[e["id"] for e in spec["entries"]]: issues.append({"kind":"manifest_entries","area":area})
line_counts={}
citation_issues=[]
citations=set()
def check_refs(text,location):
    for m in re.finditer(r"\b(ice|rust):([^\s;\x60]+?):(\d+)-(\d+)",text):
        side,rel,a,b=m.groups()
        key=m.group(0)
        citations.add(key)
        try:
            p=source(side,rel)
            if p not in line_counts: line_counts[p]=len(p.read_text().split("\n"))
            if not 1<=int(a)<=int(b)<=line_counts[p]: citation_issues.append({"location":location,"ref":key,"lines":line_counts[p]})
        except Exception as exc: citation_issues.append({"location":location,"ref":key,"error":str(exc)})
for area,spec in specs.items():
    check_refs(json.dumps(spec),f"spec-{area}.json")
    check_refs((BASE/f"{area}.md").read_text(),f"{area}.md")
extension_spec=load(E/"spec-extensions.json")
check_refs(json.dumps(extension_spec),"spec-extensions.json (draft)")
unresolved_dependencies=[{"entry":e["id"],"dependency":dep} for e in all_entries for dep in e.get("dependencies",[]) if dep not in ids]
for r in ledger:
    mapped=[x for x in r["entry_ids"].split(";") if x]
    if r["status"]=="mapped" and not mapped: issues.append({"kind":"mapped_without_entry","path":r["path"]})
    if r["status"]=="not-applicable" and not r["notes"].strip(): issues.append({"kind":"na_without_reason","path":r["path"]})
    for entry in mapped:
        if entry not in ids:
            target=pending_cross_area_references if entry in draft_ids else issues
            target.append({"kind":"ledger_unpublished_entry" if entry in draft_ids else "ledger_unknown_entry","path":r["path"],"entry":entry})
for category,rows in surface_data.items():
    for r in rows:
        if r["status"] not in ["present","pending"] and not r["entry_id"]: issues.append({"kind":"surface_missing_entry","category":category,"name":r["name"]})
        for entry in re.split(r"[;,]\s*",r["entry_id"]):
            if entry and entry not in ids:
                target=pending_cross_area_references if entry in draft_ids else issues
                target.append({"kind":"surface_unpublished_entry" if entry in draft_ids else "surface_unknown_entry","category":category,"entry":entry})
status_counts=dict(Counter(r["status"] for r in ledger))
surface_counts={k:{"total":len(v),"pending":sum(r["status"]=="pending" for r in v)} for k,v in surface_data.items()}
actual={"ledger_rows":len(ledger),"mapped":status_counts.get("mapped",0),"not_applicable":status_counts.get("not-applicable",0),"pending":status_counts.get("pending",0),"entry_count":len(all_entries),"first_pass_claims":sum(len(m.get("claims",[])) for m in manifests.values()),"surface_counts":surface_counts}
for k,v in actual.items():
    if progress.get(k)!=v: issues.append({"kind":"progress_mismatch","field":k,"recorded":progress.get(k),"actual":v})
source_drift=[]
hashes={"ice":{},"rust":{}}
supplemental=[r for m in manifests.values() for r in m.get("supplemental_ledger",[])]
for r in [*tracked,*supplemental]:
    try:
        current=digest(source("ice",r["path"]))
        hashes["ice"][r["path"]]=current
        if r.get("sha256") and current!=r["sha256"]: source_drift.append({"side":"ice","path":r["path"],"baseline":r["sha256"],"actual":current})
    except Exception as exc: source_drift.append({"side":"ice","path":r["path"],"error":str(exc)})
for item in load(E/"rust-tracked-files.json"):
    rel=item if isinstance(item,str) else item["path"]
    try:
        p=source("rust",rel)
        if p.is_file(): hashes["rust"][rel]=digest(p)
    except Exception as exc: issues.append({"kind":"rust_hash","path":rel,"error":str(exc)})
read_drift=[]
for ref,item in read_coverage.items():
    side,rel=ref.split(":",1)
    try:
        current=digest(source(side,rel))
        if item.get("sha256") and current!=item["sha256"]: read_drift.append({"ref":ref,"recorded":item["sha256"],"actual":current})
    except Exception as exc: read_drift.append({"ref":ref,"error":str(exc)})
identities={side:{"root":str(root),"head":git(root,"rev-parse","HEAD").strip(),"branch":git(root,"branch","--show-current").strip(),"status":git(root,"status","--short","--untracked-files=normal")} for side,root in ROOTS.items()}
for side in ROOTS:
    original=baseline["ts" if side=="ice" else side]
    for k in ["head","branch"]:
        if identities[side][k]!=original[k]: issues.append({"kind":"identity_drift","side":side,"field":k,"baseline":original[k],"actual":identities[side][k]})
if identities["ice"]["status"]!=baseline["ts"]["status"]: issues.append({"kind":"typescript_status_drift","baseline":baseline["ts"]["status"],"actual":identities["ice"]["status"]})
artifact_files=[BASE/"README.md",BASE/"ledger.csv",E/"progress.json",E/"execution-policy.json",E/"baseline.json",E/"read-coverage.json",*sorted((BASE/"surfaces").glob("*.csv"))]
artifact_files.extend(BASE/f"{a}.md" for a in areas)
artifact_files.extend(E/f"spec-{a}.json" for a in [*areas,"extensions"])
report={"captured_at":datetime.now(timezone.utc).isoformat(),"initial_baseline_rows":len(tracked),"supplemental_rows":len(supplemental),"counts":actual,"identities":identities,"areas":{a:{"entries":len(s["entries"]),"statuses":dict(Counter(e["status"] for e in s["entries"]))} for a,s in specs.items()},"extensions_draft_entries":len(extension_spec["entries"]),"citation_count":len(citations),"citation_issues":citation_issues,"source_drift":source_drift,"recorded_read_hash_drift":read_drift,"issues":issues,"pending_cross_area_references":pending_cross_area_references,"unresolved_dependency_ids":unresolved_dependencies,"resume_file_preexisted":(E/"RESUME.md").exists(),"artifact_hashes":{str(p.relative_to(BASE)):digest(p) for p in artifact_files},"checks_note":"Structural parsing, hashes, reference bounds and persisted-state reconciliation only; not a new semantic re-audit of certified entries."}
dump(E/f"resume-integrity-{STAMP}.json",report)
dump(E/f"resume-source-identities-{STAMP}.json",{"captured_at":report["captured_at"],"identities":identities,"hashes":hashes,"source_drift":source_drift,"note":"Historical baseline preserved. These hashes identify observed bytes; source reads require their own range evidence."})
print(json.dumps({"evidence":str(E/f"resume-integrity-{STAMP}.json"),"source_manifest":str(E/f"resume-source-identities-{STAMP}.json"),"counts":actual,"source_drift":source_drift,"recorded_read_hash_drift":read_drift,"issue_count":len(issues),"issues":issues[:30],"citation_issue_count":len(citation_issues),"citation_issues":citation_issues[:30],"unpublished_cross_area_reference_count":len(pending_cross_area_references),"unresolved_dependency_count":len(unresolved_dependencies),"hashed_files":{k:len(v) for k,v in hashes.items()}},indent=2))
