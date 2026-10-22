"""Validate the complete documentation artifact and frozen sources; no application execution."""
import ast, csv, hashlib, json, re, subprocess, sys
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path

BASE=Path(__file__).resolve().parent.parent
E=BASE/"evidence"
ROOT=BASE.parents[1]
ROOTS={"ice":Path("/home/mewtwo/Zks/ice"),"rust":ROOT,"rust-active":Path("/home/mewtwo/Zykairotis/ice-rust")}
AREAS=["ai","agent","tools","session","settings","extensions","rpc-remote","tui","subagents","safe-verify","observability","cognee","blackhole","providers-auth","cli-packaging","misc"]
phase=sys.argv[1] if len(sys.argv)>1 else "original"
if phase not in ("original","active"): raise ValueError("original or active phase required")
issues=[]; warnings=[]; refs={}; source_cache={}
def load(name): return json.loads((E/name).read_text())
def digest(path): return hashlib.sha256(path.read_bytes()).hexdigest()
def problem(kind,**kw): issues.append({"kind":kind,**kw})
def table(path,columns):
    with path.open(newline="") as f:
        reader=csv.DictReader(f,strict=True)
        if reader.fieldnames!=columns: problem("csv_header",path=str(path),actual=reader.fieldnames)
        rows=list(reader)
    for i,r in enumerate(rows):
        if None in r or any(v is None for v in r.values()): problem("csv_record",path=str(path),row=i+2)
    return rows
def source(side,rel):
    p=(ROOTS[side]/rel).resolve()
    if not p.is_relative_to(ROOTS[side].resolve()): raise ValueError("source boundary")
    if p not in source_cache:
        raw=p.read_bytes(); text=raw.decode("utf8")
        source_cache[p]={"sha256":hashlib.sha256(raw).hexdigest(),"lines":len(text.splitlines()),"split_lines":len(text.split("\n")),"text":text}
    if side=="rust-active":
        row=next((r for r in load("active-checkout-source-map.json")["rows"] if r.get("active_path")==rel),None)
        if not row or not row.get("active",{}).get("present") or row["active"]["sha256"]!=source_cache[p]["sha256"]: raise ValueError("already-active citation source hash differs from fixed map")
    return source_cache[p]
REF=re.compile(r"\b(ice|rust(?:-active)?):([A-Za-z0-9_./@+\-]+):(\d+)(?:[-–](\d+))?((?:,\s*\d+(?:[-–]\d+)?)*)")
def check_refs(text,location):
    for m in REF.finditer(text):
        side,rel,a,b,tail=m.groups()
        ranges=[(int(a),int(b or a))]
        for extra in tail.split(","):
            if extra.strip():
                z=re.split("[-–]",extra.strip()); ranges.append((int(z[0]),int(z[-1])))
        if tail: problem("unexpanded_citation",location=location,ref=m.group())
        key=m.group(); refs.setdefault(key,location)
        try:
            actual=source(side,rel)
            for a,b in ranges:
                if not 1<=a<=b<=actual["split_lines"]: problem("citation_bounds",location=location,ref=key,lines=actual["lines"])
                elif b>actual["lines"]: warnings.append({"kind":"terminal_empty_line_citation","location":location,"ref":key,"physical_lines":actual["lines"]})
        except Exception as exc: problem("citation_source",location=location,ref=key,error=str(exc))
def walk(x,location):
    if isinstance(x,str): check_refs(x,location)
    elif isinstance(x,list):
        for i,v in enumerate(x): walk(v,f"{location}[{i}]")
    elif isinstance(x,dict):
        for k,v in x.items(): walk(v,f"{location}.{k}")
specs={a:load(f"spec-{a}.json") for a in AREAS}
manifests={a:load(a+".json") for a in AREAS}
entries=[dict(e,area=a) for a,s in specs.items() for e in s["entries"]]
counts=Counter(e["id"] for e in entries); ids=set(counts)
for id,n in counts.items():
    if n!=1: problem("duplicate_entry_id",id=id,count=n)
required=["id","title","status","source","what","mechanism","interfaces","rust","port","priority","priority_reason"]
for area,spec in specs.items():
    md=(BASE/(area+".md")).read_text()
    if not spec.get("title") or not spec.get("introduction") or not md.startswith("# "+spec.get("title","")+"\n"): problem("area_header_metadata",area=area)
    expected=[e["id"] for e in spec["entries"]]
    if re.findall(r"^## ([A-Z]+-\d+) - ",md,re.M)!=expected: problem("markdown_entry_order",area=area)
    if len(re.findall(r"^### [1-9]\. ",md,re.M))!=9*len(expected): problem("nine_markdown_fields",area=area)
    manifest=manifests[area]
    if [e["id"] for e in manifest["entries"]]!=expected: problem("manifest_entry_order",area=area)
    if manifest.get("dependency_requests"): problem("unresolved_dependency_request",area=area,count=len(manifest["dependency_requests"]))
    mm={e["id"]:e for e in manifest["entries"]}
    for e in spec["entries"]:
        for k in required:
            if not isinstance(e.get(k),str) or not e[k].strip(): problem("empty_field",id=e["id"],field=k)
        if e["status"] not in ("missing","partial","different-by-design"): problem("invalid_status",id=e["id"])
        if e["priority"] not in ("P0","P1","P2","P3"): problem("invalid_priority",id=e["id"])
        if not isinstance(e.get("size_min"),int) or not isinstance(e.get("size_max"),int) or not 0<=e["size_min"]<=e["size_max"]: problem("invalid_size",id=e["id"])
        if not isinstance(e.get("dependencies"),list): problem("missing_dependencies",id=e["id"])
        if not re.search(r"\brust:[A-Za-z0-9_./@+\-]+:\d+",e["rust"]): problem("missing_native_anchor",id=e["id"])
        for target in e.get("dependencies",[]):
            if target not in ids: problem("unknown_dependency",id=e["id"],target=target)
        for k in ("title","status","priority","size_min","size_max","dependencies"):
            if mm.get(e["id"],{}).get(k)!=e.get(k): problem("manifest_field_mismatch",id=e["id"],field=k)
        for rel in e.get("dependency_relations",[]):
            if rel["entry_id"] not in ids: problem("unknown_related_id",id=e["id"],target=rel["entry_id"])
    walk(spec,"spec-"+area+".json")
    check_refs(md,area+".md")
    for c in manifest.get("claims",[]): walk(c,area+".json.claim")
    for s in manifest.get("surfaces",[]): walk(s.get("supporting_references",{}),area+".json.surface.supporting_references")
ledger=table(BASE/"ledger.csv",["path","lines","area","entry_ids","status","notes"])
if len({r["path"] for r in ledger})!=len(ledger): problem("duplicate_ledger_path")
for r in ledger:
    if r["status"] not in ("mapped","not-applicable"): problem("unfinished_ledger",path=r["path"],status=r["status"])
    mapped=[x for x in r["entry_ids"].split(";") if x]
    if r["status"]=="mapped" and not mapped: problem("ledger_without_id",path=r["path"])
    if r["status"]=="not-applicable" and not r["notes"].strip(): problem("na_without_reason",path=r["path"])
    for id in mapped:
        if id not in ids: problem("ledger_unknown_id",path=r["path"],id=id)
    if not r["lines"].isdigit() or not r["area"]: problem("ledger_schema",path=r["path"])
original=load("tracked-files.json")
if set(r["path"] for r in original)-set(r["path"] for r in ledger): problem("lost_original_ledger_rows")
surfaces={}
for p in sorted((BASE/"surfaces").glob("*.csv")):
    rows=table(p,["name","ice_path:line","rust_path:line_or_blank","status","entry_id"]);surfaces[p.stem]=rows
    for r in rows:
        if r["status"] not in ("present","partial","missing","different-by-design"): problem("unfinished_surface",category=p.stem,name=r["name"],status=r["status"])
        if r["status"]!="present" and not r["entry_id"]: problem("surface_without_id",category=p.stem,name=r["name"])
        for id in re.split("[;,]\\s*",r["entry_id"]):
            if id and id not in ids: problem("surface_unknown_id",category=p.stem,name=r["name"],id=id)
        for side,key in (("ice","ice_path:line"),("rust","rust_path:line_or_blank")):
            value=r[key]
            if not value: continue
            if not re.fullmatch(r"[A-Za-z0-9_./@+\-]+:\d+",value): problem("surface_anchor_format",category=p.stem,name=r["name"],value=value)
            else: check_refs(side+":"+value,"surface:"+p.stem+":"+r["name"])
candidates=load("surface-candidates.json"); candidate_ids={c["candidate_id"] for c in candidates}
mapped_candidates={x for m in manifests.values() for s in m.get("surfaces",[]) for x in s.get("candidate_ids",[])}
excluded_candidates={x["candidate_id"] for m in manifests.values() for x in m.get("candidate_exclusions",[])}
for id in sorted(candidate_ids-mapped_candidates-excluded_candidates): problem("pending_candidate",id=id)
for id in sorted(mapped_candidates&excluded_candidates): problem("candidate_mapping_exclusion_overlap",id=id)
for id in sorted((mapped_candidates|excluded_candidates)-candidate_ids): problem("invented_candidate_id",id=id)
for m in manifests.values():
    for x in m.get("candidate_exclusions",[]):
        if not x.get("reason","").strip(): problem("candidate_exclusion_without_reason",id=x["candidate_id"])
coverage_document=load("read-coverage.json")
coverage=coverage_document.get("files",coverage_document)
coverage_policies=coverage_document.get("coverage_policies",[])
validated_policy_ranges={}
read_drift=[]; source_drift=[]; ledger_coverage=[]
def merged(ranges):
    out=[]
    for a,b in sorted(ranges):
        if out and a<=out[-1][1]+1: out[-1][1]=max(b,out[-1][1])
        else: out.append([a,b])
    return out
def missing(n,ranges,excluded):
    bounds=merged([*ranges,*[[x,x] for x in excluded]]); cursor=1; gaps=[]
    for a,b in bounds:
        a=max(1,a); b=min(n,b)
        if b<cursor: continue
        if a>cursor: gaps.append([cursor,a-1])
        cursor=max(cursor,b+1)
    if cursor<=n: gaps.append([cursor,n])
    return gaps
for policy in coverage_policies:
    key=policy.get("path","")
    try:
        if policy.get("kind")!="generated-data-group" or not key.startswith("ice:"): raise ValueError("Unsupported coverage policy")
        rule=policy["rule"]
        if rule["path"]!="docs/ice-gap/requirements.md" or rule["line"]!=23 or digest(BASE/"requirements.md")!=rule["sha256"]: raise ValueError("Generated policy rule identity mismatch")
        if "Generated files still get one ledger row each" not in (BASE/"requirements.md").read_text().splitlines()[22]: raise ValueError("Generated grouping not authorized")
        actual=source("ice",key.split(":",1)[1])
        if actual["sha256"]!=policy["sha256"] or coverage[key]["sha256"]!=policy["sha256"]: raise ValueError("Generated file identity mismatch")
        if any(id not in ids for id in policy["entry_ids"]): raise ValueError("Generated policy references unknown entry")
        for role in ("generator","consumer"):
            proof=policy[role];side,rel=proof["path"].split(":",1);s=source(side,rel);c=coverage[proof["path"]]
            if not proof.get("fully_read") or proof["sha256"]!=s["sha256"] or c["sha256"]!=s["sha256"]: raise ValueError(role+" hash/read proof missing")
            if missing(s["lines"],c["ranges"],[]): raise ValueError(role+" is not fully source-read")
        proof=policy["evidence"]
        if digest(ROOT/proof["path"])!=proof["sha256"]: raise ValueError("Generated policy evidence hash mismatch")
        ranges=policy["not_read_ranges"]
        if any(not 1<=a<=b<=actual["lines"] for a,b in ranges): raise ValueError("Generated policy bounds")
        if any(a<=d and c<=b for a,b in ranges for c,d in coverage[key]["ranges"]): raise ValueError("Generated grouped value range falsely credited")
        validated_policy_ranges[key]=ranges
    except Exception as exc: problem("generated_policy_invalid",path=key,error=str(exc))
for proof in coverage_document.get("identical_copy_proofs",[]):
    try:
        source_key=proof["source"];copy_key=proof["copy"]
        side,rel=source_key.split(":",1);origin=source(side,rel)
        copy_side,copy_rel=copy_key.split(":",1);copy=source(copy_side,copy_rel)
        if origin["sha256"]!=copy["sha256"] or origin["sha256"]!=proof["source_sha256"] or copy["sha256"]!=proof["copy_sha256"]: raise ValueError("Copy bytes differ")
        if digest(E/proof["proof_artifact"])!=proof["proof_artifact_sha256"]: raise ValueError("Original copy proof changed")
        if coverage[copy_key].get("read_credit_kind")!="derived-byte-identical-copy": raise ValueError("Copy credit mislabeled")
        for a,b in coverage[copy_key]["ranges"]:
            if missing(b,[[1,a-1],*coverage[source_key]["ranges"]],[]): raise ValueError("Copy range exceeds source credit")
    except Exception as exc: problem("identical_copy_proof_invalid",path=proof.get("copy"),error=str(exc))
for key,r in coverage.items():
    if not isinstance(r,dict) or ":" not in key: continue
    side,rel=key.split(":",1)
    if side not in ROOTS: problem("read_unknown_repository",path=key);continue
    try:
        actual=source(side,rel)
        if not r.get("sha256"): problem("read_hash_missing",path=key)
        elif actual["sha256"]!=r["sha256"]: read_drift.append({"path":key,"recorded":r["sha256"],"actual":actual["sha256"]})
        excluded=r.get("excluded_redacted_lines",[])+r.get("excluded_unverified_lines",[])
        for x in excluded:
            if any(a<=x<=b for a,b in r.get("ranges",[])): problem("excluded_line_credited",path=key,line=x)
    except Exception as exc: problem("read_source_error",path=key,error=str(exc))
for r in ledger:
    if r["status"]=="not-applicable": continue
    key="ice:"+r["path"]
    try:
        actual=source("ice",r["path"])
        c=coverage.get(key,{})
        excluded=c.get("excluded_redacted_lines",[])+c.get("excluded_unverified_lines",[])
        grouped=validated_policy_ranges.get(key,[])
        gaps=missing(actual["lines"],[*c.get("ranges",c.get("visible_ranges",[])),*grouped],excluded)
        if gaps: problem("unread_required_source",path=r["path"],ranges=gaps)
        ledger_coverage.append({"path":r["path"],"physical_lines":actual["lines"],"excluded_lines":excluded,"policy_not_read_ranges":grouped,"ordinary_missing_ranges":gaps})
    except Exception as exc: problem("required_source_error",path=r["path"],error=str(exc))
for r in [*original,*[x for m in manifests.values() for x in m.get("supplemental_ledger",[])]]:
    if not r.get("sha256"): continue
    try:
        actual=digest(ROOTS["ice"]/r["path"])
        if actual!=r["sha256"]: source_drift.append({"path":r["path"],"baseline":r["sha256"],"actual":actual})
    except Exception as exc: source_drift.append({"path":r["path"],"error":str(exc)})
fpc=load("qa-first-pass-coverage.json")
if len(fpc["claims"])!=160 or not fpc.get("coverage_complete") or fpc.get("uncovered_claims") or fpc.get("follow_up_checks"): problem("first_pass_incomplete")
for c in fpc["claims"]:
    walk(c.get("evidence",[]),"first-pass:"+c["id"]+":evidence")
    check_refs(c.get("finding",""),"first-pass:"+c["id"]+":finding")
    if c.get("draft_dependent") or not c.get("finding") or not c.get("disposition"): problem("first_pass_unresolved",id=c["id"])
    for id in c.get("entry_ids",[]):
        if id not in ids: problem("first_pass_unknown_id",claim=c["id"],entry=id)
graph=load("dependency-graph.json")
if {n["id"] for n in graph["nodes"]}!=ids: problem("dependency_graph_node_coverage")
expected_edges={(e["id"],r["entry_id"],r["kind"]) for e in entries for r in e.get("dependency_relations",[])}
actual_edges={(e["from"],e["to"],e["kind"]) for e in graph["edges"]}
if expected_edges!=actual_edges: problem("dependency_graph_edge_coverage",missing=len(expected_edges-actual_edges),extra=len(actual_edges-expected_edges))
order=table(E/"porting-order.csv",["order","group","entry_id","area","priority","prerequisites","related_entries"])
if len(order)!=len(ids) or {r["entry_id"] for r in order}!=ids: problem("porting_order_coverage")
positions={r["entry_id"]:int(r["order"]) for r in order}
groups={r["entry_id"]:r["group"] for r in order}
for a,b,k in actual_edges:
    if k=="prerequisite" and groups.get(a)!=groups.get(b) and positions.get(b,0)>=positions.get(a,0): problem("porting_order_violation",entry=a,prerequisite=b)
defects=load("rust-defects.json")
for d in defects:
    for k in ("id","title","severity","evidence_level","behavior","technical_explanation","provenance"):
        if not d.get(k): problem("incomplete_defect",id=d.get("id"),field=k)
    if "pending" in d.get("provenance_status",""): problem("pending_defect_provenance",id=d["id"])
    for id in d.get("entry_ids",[]):
        if id not in ids: problem("defect_unknown_entry",defect=d["id"],entry=id)
    for ref in d.get("sources",[]): check_refs(ref,d["id"])
readme=(BASE/"README.md").read_text()
check_refs(readme,"README.md")
index_ids=re.findall(r"^\| \[([A-Z]+-\d+)\]\(",readme,re.M)
if Counter(index_ids)!=Counter(ids): problem("readme_entry_index",actual=len(index_ids),expected=len(ids))
qa=load("qa-unnamed-exports-2026-10-20.json")
expected_exports={(r["ts"],r["symbol"]):r for rows in qa["symbols"].values() for r in rows}
export_review=load("qa-unnamed-exports-dispositions.json")
dispositions=export_review.get("dispositions",[])
export_keys=[(r.get("ts"),r.get("symbol")) for r in dispositions]
if set(export_keys)!=set(expected_exports) or len(export_keys)!=len(set(export_keys)): problem("export_coverage_mismatch",expected=len(expected_exports),actual=len(export_keys))
if not export_review.get("complete") or export_review.get("summary",{}).get("unresolved")!=0: problem("export_review_incomplete")
byid={e["id"]:e for e in entries}
unnamed=[]
for r in dispositions:
    key=(r.get("ts"),r.get("symbol"))
    if not r.get("reason") or not r.get("source_review") or not r.get("disposition"): problem("export_disposition_missing_reason",key=key)
    if key in expected_exports and r.get("kind")!=expected_exports[key]["kind"]: problem("export_kind_changed",key=key)
    target_ids=r.get("entry_ids",[])
    for id in target_ids:
        if id not in byid: problem("export_unknown_entry",key=key,id=id)
    if target_ids and not any(re.search(r"(?<![A-Za-z0-9_])"+re.escape(r["symbol"])+r"(?![A-Za-z0-9_])",byid.get(id,{}).get("source","")+" "+byid.get(id,{}).get("interfaces","")) for id in target_ids):
        unnamed.append({"symbol":r["symbol"],"ts":r["ts"],"entry_ids":target_ids})
if unnamed: problem("unnamed_owned_export",count=len(unnamed),records=unnamed)
spacing=load("spacing-validation.json")
for k in ("citations_byte_identical","identifiers_byte_identical","protected_spans_byte_identical"):
    if spacing.get(k) is not True: problem("spacing_preservation_failed",check=k)
if len(spacing.get("approval",{}).get("reviewed_change_indexes",[]))<50: problem("spacing_review_too_small")
if any(spacing.get(k) for k in ("protected_span_mismatches","identifier_diffs","citation_diffs")): problem("spacing_unresolved_differences")
residual=load("spacing-residual-validation.json")
if residual.get("passed") is not True: problem("residual_spacing_not_passed")
if residual.get("applied") is not True or residual.get("non_prose_structure_byte_identical") is not True: problem("residual_spacing_not_applied_with_structure_proof")
if residual.get("tool_sha256")!=digest(BASE/"tools/fix-spacing.mjs"): problem("residual_spacing_tool_changed")
for field,name in (("dry_run_sha256","spacing-residual-dry-run.json"),("approval_sha256","lane-residual-spacing-review.json")):
    if residual.get(field)!=digest(E/name): problem("residual_spacing_bound_evidence_changed",artifact=name)
residual_approval=load("lane-residual-spacing-review.json")
if residual_approval.get("approved") is not True or residual_approval.get("reviewed_change_indexes")!=[c["index"] for c in residual.get("changes",[])]: problem("residual_spacing_review_incomplete")
for k in ("citations_byte_identical","identifiers_byte_identical","protected_spans_byte_identical"):
    if residual.get(k) is not True: problem("residual_spacing_preservation_failed",check=k)
if residual.get("remaining_real_prose_glue")!=0: problem("residual_spacing_remaining",count=residual.get("remaining_real_prose_glue"))
spacing_check=subprocess.run(["node",str(BASE/"tools/fix-spacing.mjs"),"residual-check"],capture_output=True,text=True)
if spacing_check.returncode: problem("current_residual_spacing_failed",returncode=spacing_check.returncode,stdout=spacing_check.stdout[:5000],stderr=spacing_check.stderr[:5000])
for e in entries:
    opening=e["mechanism"].strip().split("\n\n")[0]
    if not opening or re.match(r"^(?:\x60{3}|#{1,6}\s|[-*]\s|ice:|rust:|Sources?:|See |This entry (?:describes|covers|documents))",opening,re.I): problem("mechanism_lacks_behavioral_opening",id=e["id"])
if not re.search(r"UTF.?16",byid["AGENT-057"]["mechanism"],re.I): problem("lexer_precision_missing")
if "captureSession" not in byid["COG-046"]["mechanism"]: problem("pending_prompt_gate_precision_missing")
for id in ("OBS-017","COG-021"):
    if "createObserveCogneeSource" not in byid[id]["source"]: problem("observe_factory_name_missing",id=id)
def git(root,*args): return subprocess.check_output(["git","--no-optional-locks","-C",str(root),*args],text=True)
identities={side:{"root":str(root),"head":git(root,"rev-parse","HEAD").strip(),"branch":git(root,"branch","--show-current").strip(),"status":git(root,"status","--short","--untracked-files=normal")} for side,root in ROOTS.items() if side in ("ice","rust")}
if any(ref.startswith("rust-active:") for ref in refs):
    active_expected=load("active-checkout-source-map.json")["identities"]["active"]
    for k,args in (("head",("rev-parse","HEAD")),("branch",("branch","--show-current"))):
        if git(ROOTS["rust-active"],*args).strip()!=active_expected[k]: problem("active_alias_identity_drift",field=k)
baseline=load("baseline.json")
native_expected=baseline["rust"] if phase=="original" else load("active-checkout-source-map.json")["identities"]["active"]
for side,old in (("ice",baseline["ts"]),("rust",native_expected)):
    for k in ("head","branch"):
        if identities[side][k]!=old[k]: problem("identity_drift",side=side,field=k,expected=old[k],actual=identities[side][k])
if identities["ice"]["status"]!=baseline["ts"]["status"]: problem("typescript_status_drift",expected=baseline["ts"]["status"],actual=identities["ice"]["status"])
if git(ROOT,"diff","--name-only","HEAD","--","crates","scripts","Cargo.toml","Cargo.lock").strip(): problem("native_source_worktree_changed")
progress=load("progress.json")
actual={"ledger_rows":len(ledger),"mapped":sum(r["status"]=="mapped" for r in ledger),"not_applicable":sum(r["status"]=="not-applicable" for r in ledger),"pending":sum(r["status"]=="pending" for r in ledger),"entry_count":len(entries),"first_pass_claims":sum(len(m.get("claims",[])) for m in manifests.values()),"surface_counts":{k:{"total":len(v),"pending":sum(r["status"]=="pending" for r in v)} for k,v in surfaces.items()}}
for k,v in actual.items():
    if progress.get(k)!=v: problem("progress_count_mismatch",field=k,recorded=progress.get(k),actual=v)
if source_drift: problem("typescript_source_drift",count=len(source_drift))
if read_drift: problem("read_hash_drift",count=len(read_drift))
if phase=="active":
    expected={"README.md","requirements.md","ledger.csv","surfaces","evidence","tools",*[a+".md" for a in AREAS]}
    extra={p.name for p in BASE.iterdir()}-expected
    if extra: problem("top_level_residue",paths=sorted(extra))
    outside=[str(p.relative_to(BASE)) for p in BASE.rglob("*") if p.is_file() and p.suffix in (".mjs",".py") and not p.is_relative_to(BASE/"tools")]
    if outside: problem("helper_outside_tools",paths=outside)
    residue=[str(p.relative_to(BASE)) for p in BASE.rglob("*") if p.name in (".venv","node_modules","__pycache__") or p.suffix==".pyc"]
    if residue: problem("tool_residue",paths=residue)
    if progress.get("complete"):
        for area in AREAS:
            for kind,data in (("spec",specs[area]),("manifest",manifests[area])):
                if data.get("complete") is not True or data.get("draft_complete") is not True or data.get("accepted_phase")!="active-source-verified": problem("completion_metadata_mismatch",area=area,artifact_kind=kind)
    relocation=load("active-relocation-verification.json")
    if not relocation.get("all_rust_citations_mapped_and_checked"): problem("active_citation_relocation_incomplete")
    helper=load("relocated-helper-validation.json")
    if helper.get("issues") or not helper.get("safe_execution_complete"): problem("relocated_helpers_incomplete")
    if git(ROOT,"remote","get-url","origin").strip()!=native_expected["origin"]: problem("active_origin_drift")
    fixed=next((d for d in defects if d["id"]=="RUST-045"),{})
    if fixed.get("active_status")!="fixed": problem("active_rebrand_defect_status_missing")
    for rel,line in (("crates/ice-cli/src/prebuilt_release.rs",11),("crates/ice-cli/src/changelog.rs",27)):
        text=source("rust",rel)["text"].splitlines()[line-1]
        if "zyk-void/ice-rust" not in text: problem("active_repository_constant_not_fixed",path=rel,line=line)
report={"schema_version":1,"captured_at":datetime.now(timezone.utc).isoformat(),"phase":phase,"source_only":True,"application_or_test_execution":False,"passed":not issues,"counts":actual,"areas":{a:{"entries":len(s["entries"]),"statuses":dict(Counter(e["status"] for e in s["entries"])),"priorities":dict(Counter(e["priority"] for e in s["entries"]))} for a,s in specs.items()},"candidate_counts":{"original":len(candidate_ids),"mapped":len(mapped_candidates),"excluded":len(excluded_candidates),"pending":len(candidate_ids-mapped_candidates-excluded_candidates)},"source_reference_count":len(refs),"source_file_count_checked":len(source_cache),"canonical_read_files":len(coverage),"ledger_source_coverage":ledger_coverage,"first_pass_claims":len(fpc["claims"]),"first_pass_dispositions":dict(Counter(c["disposition"] for c in fpc["claims"])),"native_defects":len(defects),"native_defect_severity":dict(Counter(re.split(r"[ ;(]",d["severity"],maxsplit=1)[0] for d in defects)),"graph":{"nodes":len(graph["nodes"]),"edges":len(graph["edges"]),"porting_order_entries":len(order)},"residual_spacing":{"proof_sha256":digest(E/"spacing-residual-validation.json"),"current_check_exit_code":spacing_check.returncode,"current_check_stdout":spacing_check.stdout},"identities":identities,"source_drift":source_drift,"recorded_read_hash_drift":read_drift,"issues":issues,"warnings":warnings,"limits":["Source reference bounds and evidence consistency are checked; runtime effects and external services remain unverified.","Explicit connector redactions and the preserved sessions.rs627 provenance conflict are excluded from visible read coverage.","Terminal empty split lines are not counted as physical source behavior."]}
(E/"final-validation.json").write_text(json.dumps(report,indent=2,ensure_ascii=False)+"\n")
print(json.dumps({"phase":phase,"passed":report["passed"],"counts":actual,"canonical_read_files":len(coverage),"source_reference_count":len(refs),"issue_count":len(issues),"issue_kinds":dict(Counter(i["kind"] for i in issues)),"first_issues":issues[:20],"warning_count":len(warnings),"full_report":str(E/"final-validation.json")},indent=2))
sys.exit(0 if report["passed"] else 1)
