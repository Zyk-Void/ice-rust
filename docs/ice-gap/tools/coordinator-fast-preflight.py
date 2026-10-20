"""Incremental closeout integrity verification. Audit evidence only; no application execution."""
import csv, hashlib, json, re, subprocess
from pathlib import Path
from datetime import datetime, timezone
from collections import Counter
BASE=Path(__file__).resolve().parent.parent
E=BASE/"evidence"
ROOTS={"ice":Path("/home/mewtwo/Zks/ice"),"rust":BASE.parents[1]}
def load(n): return json.loads((E/n).read_text())
def sha(p): return hashlib.sha256(p.read_bytes()).hexdigest()
def git(root,*a): return subprocess.check_output(["git","--no-optional-locks","-C",str(root),*a],text=True)
previous=max((p for p in E.glob("resume-integrity-*.json")),key=lambda p:p.stat().st_mtime)
pr=json.loads(previous.read_text()); since=datetime.fromisoformat(pr["captured_at"].replace("Z","+00:00"))
baseline=load("baseline.json")
source_map=load("active-checkout-source-map.json")
active_expected=source_map["identities"]["active"]
ROOTS["rust-active"]=Path(active_expected["root"])
identities={s:{"head":git(r,"rev-parse","HEAD").strip(),"branch":git(r,"branch","--show-current").strip(),"status":git(r,"status","--short","--untracked-files=normal")} for s,r in ROOTS.items()}
issues=[]
for side,key in (("ice","ts"),("rust","rust")):
 for k in ("head","branch"):
  if identities[side][k]!=baseline[key][k]: issues.append({"kind":"identity_drift","side":side,"field":k,"actual":identities[side][k]})
if identities["ice"]["status"]!=baseline["ts"]["status"]: issues.append({"kind":"typescript_dirty_list_changed","before":baseline["ts"]["status"],"after":identities["ice"]["status"]})
if git(ROOTS["rust"],"diff","--name-only","HEAD","--","crates","scripts","Cargo.toml","Cargo.lock").strip(): issues.append({"kind":"rust_tracked_source_changed"})
for k in ("head","branch"):
 if identities["rust-active"][k]!=active_expected[k]: issues.append({"kind":"active_identity_drift","field":k,"actual":identities["rust-active"][k]})
if identities["rust-active"]["status"]: issues.append({"kind":"active_worktree_not_clean","status":identities["rust-active"]["status"]})
if git(ROOTS["rust-active"],"remote","get-url","origin").strip()!=active_expected["origin"]: issues.append({"kind":"active_origin_drift"})
ancestor=subprocess.run(["git","--no-optional-locks","-C",str(ROOTS["rust-active"]),"merge-base","--is-ancestor",baseline["rust"]["head"],active_expected["head"]],capture_output=True,text=True)
if ancestor.returncode!=0: issues.append({"kind":"old_not_verified_ancestor","returncode":ancestor.returncode,"stderr":ancestor.stderr})
expected={}
for r in load("tracked-files.json"):
 if r.get("sha256"): expected["ice:"+r["path"]]=r["sha256"]
for r in source_map["rows"]:
 if r.get("old_path") and r.get("old",{}).get("sha256"): expected["rust:"+r["old_path"]]=r["old"]["sha256"]
 if r.get("active_path") and r.get("active",{}).get("sha256"): expected["rust-active:"+r["active_path"]]=r["active"]["sha256"]
coverage=load("read-coverage.json")
for key,r in coverage.get("files",coverage).items():
 if isinstance(r,dict) and r.get("sha256"): expected.setdefault(key,r["sha256"])
checked=[]; unchanged=[]; missing=[]
for key,digest in expected.items():
 side,rel=key.split(":",1)
 if side not in ROOTS: continue
 p=(ROOTS[side]/rel).resolve()
 if not p.is_relative_to(ROOTS[side].resolve()): raise ValueError("Invalid source boundary")
 try: st=p.stat()
 except FileNotFoundError: missing.append(key); continue
 if st.st_mtime>since.timestamp():
  actual=sha(p); checked.append({"path":key,"expected":digest,"actual":actual,"match":actual==digest,"mtime":datetime.fromtimestamp(st.st_mtime,timezone.utc).isoformat()})
  if actual!=digest: issues.append({"kind":"source_drift","path":key,"expected":digest,"actual":actual})
 else: unchanged.append(key)
def table(p,header):
 with p.open(newline="") as f:
  r=csv.DictReader(f,strict=True)
  if r.fieldnames!=header: issues.append({"kind":"csv_columns","path":str(p),"actual":r.fieldnames})
  rows=list(r)
 for i,x in enumerate(rows,2):
  if None in x or any(v is None for v in x.values()): issues.append({"kind":"csv_record","path":str(p),"line":i})
 return rows
ledger=table(BASE/"ledger.csv",["path","lines","area","entry_ids","status","notes"])
surfaces={}
for p in sorted((BASE/"surfaces").glob("*.csv")):
 rows=table(p,["name","ice_path:line","rust_path:line_or_blank","status","entry_id"])
 surfaces[p.stem]={"rows":len(rows),"pending":sum(r["status"]=="pending" for r in rows)}
areas={}
for p in sorted(E.glob("spec-*.json")):
 area=p.stem.removeprefix("spec-")
 if not (E/(area+".json")).exists(): continue
 spec=json.loads(p.read_text()); md=(BASE/(area+".md")).read_text()
 ids=[e["id"] for e in spec["entries"]]
 headings=re.findall(r"^## ([A-Z]+-\d+) - ",md,re.M)
 tail=spec.get("appendix") or spec["entries"][-1]["priority_reason"]
 ok=headings==ids and len(re.findall(r"^### [1-9]\. ",md,re.M))==len(ids)*9 and md.rstrip().endswith(tail.rstrip())
 if not ok: issues.append({"kind":"published_markdown_incomplete","area":area})
 areas[area]={"entries":len(ids),"complete_entry_tail":ok}
progress=load("progress.json")
counts={"ledger_rows":len(ledger),"mapped":sum(r["status"]=="mapped" for r in ledger),"not_applicable":sum(r["status"]=="not-applicable" for r in ledger),"pending":sum(r["status"]=="pending" for r in ledger),"published_entries":sum(x["entries"] for x in areas.values())}
for k in ("ledger_rows","mapped","not_applicable","pending"):
 if progress.get(k)!=counts[k]: issues.append({"kind":"progress_mismatch","field":k,"recorded":progress.get(k),"actual":counts[k]})
now=datetime.now(timezone.utc)
report={"captured_at":now.isoformat(),"method":"Incremental mtime-gated hashes only; preserved original baselines and prior full integrity evidence. Real CSV/JSON parsers and complete final entry check.","previous_integrity":previous.name,"since":since.isoformat(),"identities":identities,"counts":counts,"areas":areas,"surfaces":surfaces,"known_source_records":len(expected),"newer_files_checked":checked,"not_newer_count":len(unchanged),"missing_expected_sources":missing,"issues":issues,"passed":not issues,"source_drift_note":("No drift since "+since.isoformat()) if not any(x["kind"]=="source_drift" for x in issues) else "Source drift requires reconciliation","untracked_rust_status_is_not_tracked_source_drift":True}
out=E/("resume-integrity-"+now.strftime("%Y-%m-%dT%H%M%SZ")+".json")
out.write_text(json.dumps(report,indent=2)+"\n")
print(json.dumps({"artifact":str(out),"passed":report["passed"],"counts":counts,"published_areas":len(areas),"newer_hash_checks":len(checked),"issues":issues,"missing_expected_sources":missing},indent=2))
