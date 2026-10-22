import { auditHelperEvidence } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
assertAuditOpen();
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import {fileURLToPath} from "node:url";
const base=auditHelperEvidence;
const read=n=>JSON.parse(fs.readFileSync(path.join(base,n),"utf8"));
const hash=n=>crypto.createHash("sha256").update(fs.readFileSync(path.join(base,n))).digest("hex");
const [mode="preview"]=process.argv.slice(2);
if(!["preview","write"].includes(mode))throw Error("Expected preview or write");
const expected={"spec-tui.json":"6348be7e858ee5d22c78cfe04f6fb003b00b9f9ed55b66c0271d78995c38254d","proposal-tui.json":"3d592ed0e1c290b2f50dd5a34a301b680e196161408c8fa93bbe4fc03cda43bd","tui-source-review.json":"d625c23ceade82b50d3b72837bc9440d757b00e84a3f8528291cd1b08a3ad1ca","tui-closeout-candidate-dispositions.json":"cb06a7f00643113a3707336796c42236768fa05eb9af27fe77591596b3667c7c"};
for(const[n,h]of Object.entries(expected))if(hash(n)!==h)throw Error("Stale input "+n);
const spec=read("spec-tui.json"),old=read("proposal-tui.json"),terminal=read("proposal-tui-terminal.json"),review=read("tui-source-review.json"),close=read("tui-closeout-source-review.json"),terminalReview=read("tui-terminal-source-review.json"),extra=read("tui-closeout-final-source-supplement.json"),plans=read("tui-closeout-candidate-dispositions.json");
const candidates=read("candidates-tui.json"),tracked=read("tracked-files.json").filter(r=>r.area==="tui"),entryMap=new Map(spec.entries.map(e=>[e.id,e])),candidateMap=new Map(candidates.map(c=>[c.candidate_id,c]));
const nearest=e=>e.rust.match(/rust:([A-Za-z0-9_./@+\-]+:\d+(?:-\d+)?)/)?.[1]??"";
const surfaces=[...old.surfaces,...terminal.surfaces];
const candidate_exclusions=[...old.candidate_exclusions,...terminal.candidate_exclusions];
for(const d of plans.dispositions){
 const c=candidateMap.get(d.candidate_id);if(!c)throw Error("Unknown candidate "+d.candidate_id);
 if(d.disposition==="excluded"){candidate_exclusions.push({candidate_id:c.candidate_id,reason:d.reason,entry_ids:d.entry_ids,ice_ref:c.path+":"+c.line});continue;}
 const e=entryMap.get(d.entry_id);if(!e)throw Error("Unknown entry "+d.entry_id);
 surfaces.push({category:c.category,name:c.name,ice_ref:c.path+":"+c.line,rust_ref:nearest(e),status:d.status??e.status,entry_id:e.id,candidate_ids:[c.candidate_id],notes:d.reason+" Source candidate body "+c.line+"–"+c.end_line+"; exact mechanism and native differences are in "+e.id+"."});
}
for(const s of plans.manual_surfaces){const e=entryMap.get(s.entry_id);if(!e)throw Error("Unknown manual entry");surfaces.push({...s,rust_ref:s.rust_ref??nearest(e),status:s.status??e.status});}
const mapped=new Set(surfaces.flatMap(s=>s.candidate_ids??(s.candidate_id?[s.candidate_id]:[]))),excluded=new Set(candidate_exclusions.map(x=>x.candidate_id));
const unknown=[...mapped,...excluded].filter(x=>!candidateMap.has(x)),overlap=[...mapped].filter(x=>excluded.has(x)),pending=candidates.filter(c=>!mapped.has(c.candidate_id)&&!excluded.has(c.candidate_id));
if(unknown.length||overlap.length||pending.length)throw Error("Candidate closure failed "+JSON.stringify({unknown,overlap,pending}));
const sources=[
 {file:"tui-source-review.json",key:"read_evidence",records:review.read_evidence,defaultReader:"original TUI owner"},
 {file:"tui-source-review.json",key:"shared_read_evidence",records:review.shared_read_evidence,defaultReader:"delegated area owner"},
 {file:"tui-closeout-source-review.json",key:"read_evidence",records:close.read_evidence,defaultReader:"/root/tui_closeout"},
 {file:"tui-terminal-source-review.json",key:"read_records",records:terminalReview.read_records,defaultReader:"/root/audit_tools"},
 {file:"misc-coordinator-docs-review.json",key:"read_evidence",records:read("misc-coordinator-docs-review.json").read_evidence,defaultReader:"/root"},
 {file:"tui-closeout-final-source-supplement.json",key:"read_evidence",records:extra.read_evidence,defaultReader:"/root/tui_closeout"},
 {file:"tui-closeout-final-source-supplement.json",key:"shared_read_evidence",records:extra.shared_read_evidence,defaultReader:"delegated area owner"},
 {file:"tui-closeout-source-review.json",key:"status_test_metadata_correction",records:[close.status_test_metadata_correction],defaultReader:"original TUI owner"}
];
for(const f of ["tui-native-controller-supplement.json","tui-native-controller-test-supplement.json","tui-native-helper-supplement.json"])sources.push({file:f,key:"source_reads",records:read(f).source_reads,defaultReader:"/root"});
const toSet=rs=>{const s=new Set();for(const[a,b]of rs??[])for(let n=a;n<=b;n++)s.add(n);return s;};
const toRanges=s=>{const r=[];for(const n of [...s].sort((a,b)=>a-b)){if(r.length&&r.at(-1)[1]+1===n)r.at(-1)[1]=n;else r.push([n,n]);}return r;};
const blockedOf=r=>{const s=toSet(r.blocked_ranges??[]);for(const n of r.excluded_redacted_lines??[])if(Number.isInteger(n))s.add(n);for(const n of r.excluded_unverified_lines??[])if(Number.isInteger(n))s.add(n);return s;};
const byPath=new Map(),identityConflicts=[],totalConflicts=[];
for(const g of sources)g.records.forEach((r,index)=>{
 if(!r?.path||!r.sha256)throw Error("Missing read path/hash "+g.file+" "+index);
 let b=byPath.get(r.path);if(!b){b={path:r.path,sha256:r.sha256,totals:new Set(),lines:new Set(),blocked:new Set(),unverified:new Set(),items:[],baseline:[],baselineHashes:new Set()};byPath.set(r.path,b);}
 if(b.sha256!==r.sha256)identityConflicts.push({path:r.path,first:b.sha256,other:r.sha256,file:g.file,index});
 const total=r.total_lines??r.totalLines??r.line_count;if(total)b.totals.add(total);
 const rr=r.visible_ranges??r.ranges??(r.range?[r.range]:[]),bad=blockedOf(r);
 for(const n of toSet(rr))b.lines.add(n);for(const n of bad)b.blocked.add(n);
 if(r.baseline_match!==undefined)b.baseline.push(r.baseline_match);
 if(r.baseline_sha256)b.baselineHashes.add(r.baseline_sha256);
 b.items.push({artifact:g.file,artifact_sha256:hash(g.file),field:g.key,record_index:index,reader:r.read_by??r.reader??r.owner??g.defaultReader,evidence:r.evidence??null,original_ranges:rr,excluded_lines:[...bad],notes:r.notes??"",baseline_match:r.baseline_match??null});
});
for(const gap of extra.exclusion_policy.unverified_source_gaps){const b=byPath.get(gap.path);if(!b)throw Error("Missing conflict source");for(const n of gap.lines){b.blocked.add(n);b.unverified.add(n);}}
const dirty=new Set(["ice:packages/coding-agent/test/status-line.test.ts","ice:packages/tui/test/editor.test.ts"]);
const normalized=[];
for(const b of byPath.values()){
 if(b.totals.size>1)totalConflicts.push({path:b.path,totals:[...b.totals]});
 if(!b.totals.size)throw Error("Missing total metadata "+b.path);
 const total=Math.max(...b.totals);for(const n of b.blocked)b.lines.delete(n);
 const redacted=[...b.blocked].filter(n=>!b.unverified.has(n)).sort((a,b)=>a-b),unverified=[...b.unverified].sort((a,b)=>a-b);
 for(const n of b.lines)if(n<1||n>total)throw Error("Read exceeds metadata "+b.path+" "+n+" "+total);
 const rr=toRanges(b.lines),sourcesClean=b.items.map(s=>{const lines=toSet(s.original_ranges);for(const n of b.blocked)lines.delete(n);const {original_ranges,...rest}=s;return{...rest,visible_ranges:toRanges(lines),excluded_lines:[...new Set([...s.excluded_lines,...b.blocked])].sort((a,b)=>a-b)};});
 const rec={path:b.path,sha256:b.sha256,total_lines:total,ranges:rr,visible_ranges:rr,excluded_redacted_lines:redacted,excluded_unverified_lines:unverified,blocked_ranges:toRanges(b.blocked),full_file_read:b.lines.size===total,fully_read:b.lines.size===total,visible_lines:b.lines.size,read_sources:sourcesClean,source_only:true,tests_executed:false};
 if(dirty.has(b.path)){rec.baseline_match=null;rec.baseline_sha256=null;rec.baseline_identity="pre-existing-dirty-current-only";rec.notes="Already dirty at frozen baseline; original working-tree hash was not captured. Current visible source is read at this SHA. This is not newly detected drift.";}
 else {rec.baseline_match=b.baseline.includes(false)?false:b.baseline.includes(true)?true:null;if(b.baselineHashes.size===1)rec.baseline_sha256=[...b.baselineHashes][0];rec.notes="Union of exact visible source ranges credited to the listed readers; all known connector redactions and explicit unverified source gaps removed before union.";}
 if(unverified.length)rec.notes+=" Conservative source-credit conflict remains for the listed line(s); parent is reconciling the original reader evidence without recovering possibly redacted text.";
 normalized.push(rec);
}
normalized.sort((a,b)=>a.path.localeCompare(b.path));
const readMap=new Map(normalized.map(r=>[r.path,r]));
const existingLedger=new Map([...old.ledger,...terminal.ledger].map(r=>[r.path,r]));
const ledger=tracked.map(f=>{
 const oldRow=existingLedger.get(f.path),ids=new Set(oldRow?.entry_ids??[]);
 for(const s of surfaces)if(s.ice_ref.startsWith(f.path+":"))ids.add(s.entry_id);
 for(const e of spec.entries)if(e.source.includes("ice:"+f.path+":"))ids.add(e.id);
 if(f.path.endsWith("/components/index.ts"))for(const e of spec.entries)if(Number(e.id.slice(4))<200&&e.source.includes("/modes/interactive/components/"))ids.add(e.id);
 if(!ids.size)throw Error("Unmapped ledger path "+f.path);
 const rec=readMap.get("ice:"+f.path);if(!rec)throw Error("No source-read record "+f.path);
 const excludedHere=[...rec.excluded_redacted_lines,...rec.excluded_unverified_lines].filter(n=>n<=f.lines);
 return {path:f.path,entry_ids:[...ids].filter(Boolean).sort(),status:"mapped",notes:(f.path.endsWith("/components/index.ts")?"Pure component re-export barrel; behavior is assigned to the exported component contracts. ":"")+"Frozen source SHA "+rec.sha256+"; exact visible read ranges and attribution in tui-source-review.json. "+(excludedHere.length?"Known excluded line(s): "+excludedHere.join(",")+". ":"")+"All original mechanical leads for this path are mapped or explicitly excluded; no application/test execution."};
});
const requiredCoverage=tracked.map(f=>{const rec=readMap.get("ice:"+f.path),seen=toSet(rec?.ranges),bad=new Set([...(rec?.excluded_redacted_lines??[]),...(rec?.excluded_unverified_lines??[])]),missing=new Set(),unresolved=new Set();for(let n=1;n<=f.lines;n++)if(!seen.has(n)){missing.add(n);if(!bad.has(n))unresolved.add(n);}return{path:f.path,tracked_lines:f.lines,logical_lines:rec?.total_lines,sha256:rec?.sha256,visible_lines_in_tracked_basis:f.lines-missing.size,excluded_ranges:toRanges(missing),unresolved_ranges:toRanges(unresolved)};});
const unresolvedRequired=requiredCoverage.filter(r=>r.unresolved_ranges.length);
const partialTests=normalized.filter(r=>/(?:\/tests?\/|\.test\.|\/tests\.rs$)/.test(r.path)&&!r.full_file_read).map(r=>({path:r.path,total_lines:r.total_lines,ranges:r.ranges,excluded_redacted_lines:r.excluded_redacted_lines,excluded_unverified_lines:r.excluded_unverified_lines,ordinary_missing_ranges:toRanges(new Set(Array.from({length:r.total_lines},(_,i)=>i+1).filter(n=>!toSet(r.ranges).has(n)&&!new Set([...r.excluded_redacted_lines,...r.excluded_unverified_lines]).has(n))))}));
const fpc=read("qa-first-pass-coverage.json"),claims=fpc.claims.filter(c=>(c.entry_ids??[]).some(id=>id.startsWith("TUI-"))||c.id==="FPC-147").map(c=>{const out={...c,source_qa_artifact:"docs/ice-gap/evidence/qa-first-pass-coverage.json",source_qa_sha256:hash("qa-first-pass-coverage.json"),draft_dependent:false};if(c.id==="FPC-147"){out.entry_ids=[...new Set([...(c.entry_ids??[]),"TUI-078"])];out.finding=(c.finding??"")+" Complete share consumer is now audited in TUI-078, including fixed temporary HTML, gh lifecycle/cancel limits, viewer URL and native Markdown/clipboard behavior.";}if(["FPC-114","FPC-137"].includes(c.id))out.finding+=" TUI-055 now explicitly narrows native support to user-Bash streaming/cancellation; it does not infer a model-facing background registry.";return out;});
const supplementalPaths=[...new Set(normalized.filter(r=>r.path.startsWith("ice:")&&!tracked.some(f=>"ice:"+f.path===r.path)).map(r=>r.path.slice(4)))];
const supplemental_ledger=supplementalPaths.map(p=>{const r=readMap.get("ice:"+p);return{path:p,status:r.full_file_read?"source-read":r.excluded_redacted_lines.length||r.excluded_unverified_lines.length?"source-read-with-exclusions":"bounded-supporting-read",notes:r.baseline_identity??"Exact visible ranges, source SHA and reader attribution in tui-source-review.json; source only."}});
const unverified=[
 "Connector-redacted source/test lines are excluded in every normalized read record; none were recovered or treated as visible.",
 "rust:crates/ice-cli/src/interactive_tui/sessions.rs:627 has a source-credit conflict: the original TUI record omitted it while SESSION raw records claimed full read. It remains excluded pending coordinator reconciliation.",
 "status-line.test.ts and packages/tui/test/editor.test.ts were dirty at baseline without a captured original working-tree SHA. Their current bodies are source-read; neither is described as new drift.",
 "TypeBox node_modules package inspection was denied; default runtime extra-property handling remains unverified. No bypass or install was attempted.",
 "Private user theme existence/import history and real user-profile state remain outside the authorized source scope.",
 "External terminal versions, gh/auth/upload responses and actual tmux negotiation were not executed; local process construction and documented policy are the evidence."
];
const rustOnly=[...terminal.rust_only,
 {entry_ids:["TUI-003","TUI-024"],description:"Preserve native live ThemeManager switching/revision invalidation and existing three presets; missing JSON/customizer contracts do not make native themes static-only."},
 {entry_ids:["TUI-025"],description:"Preserve native copy-on-select setting and working immediate/live settings controls."},
 {entry_ids:["TUI-046","TUI-047","TUI-073"],description:"Preserve native masked API-key input, stale OAuth-result rejection, structured dialog values/permission metadata and real input placeholders."},
 {entry_ids:["TUI-074"],description:"Preserve native dequeue's expanded-text materialization and pending-image restoration; separate from the source-verified submit/follow-up failure differences."},
 {entry_ids:["TUI-072"],description:"Preserve dedicated native changelog baseline, fresh-settings monotonic writes and explicit /changelog all."},
 {entry_ids:["TUI-078","TUI-080"],description:"Preserve native share-to-clipboard fallback, direct clipboard image attachments and bounded failed-copy preview."},
 {entry_ids:["TUI-079","TUI-082"],description:"Preserve native regular/fullscreen modes and useful lane/leaf/active-operation usage details."}
];
const candidate_accounting={original:candidates.length,mapped:mapped.size,excluded:excluded.size,pending:pending.length,mechanical_review_complete:true,manual_additional_surfaces:plans.manual_surfaces.length};
const proposal={...old,area:"tui",complete:false,draft_complete:true,stage:"owner-complete-awaiting-coordinator-acceptance",entries:spec.entries.map(({id,title,status,priority,size_min,size_max,dependencies})=>({id,title,status,priority,size_min,size_max,dependencies})),ledger,supplemental_ledger,surfaces,candidate_exclusions,candidate_mapping:candidates.map(c=>({candidate_id:c.candidate_id,disposition:excluded.has(c.candidate_id)?"excluded":"mapped",entry_ids:excluded.has(c.candidate_id)?candidate_exclusions.find(e=>e.candidate_id===c.candidate_id).entry_ids:[...new Set(surfaces.filter(s=>(s.candidate_ids??[]).includes(c.candidate_id)).map(s=>s.entry_id))]})),claims,read_evidence:normalized,shared_read_evidence:[],dependency_requests:[],rust_only:rustOnly,unverified,open_questions:[],candidate_accounting,verification:{source_only:true,tests_executed:false,application_executed:false,original_ledger_rows:tracked.length,expected_entry_count:151,source_credit_conflicts:extra.exclusion_policy.unverified_source_gaps,required_coverage:requiredCoverage,coordinator_qa_required:true,final_shared_publication_owner:"/root"}};
const finalReview={...review,stage:"owner-complete-awaiting-coordinator-acceptance",owner:"/root/tui_closeout",source_only:true,read_evidence:normalized,shared_read_evidence:[],findings:[...review.findings,...close.findings,...Object.entries(extra.finding_handoff).map(([topic,finding])=>({topic,finding}))],test_identities:[...review.test_identities,...close.test_identities].filter((r,i,a)=>a.findIndex(x=>x.path===r.path)===i),unread_notes:[],unverified,candidate_count:923,closeout:{spec_sha256:hash("spec-tui.json"),candidate_accounting,source_input_artifacts:sources.map(g=>({path:"docs/ice-gap/evidence/"+g.file,sha256:hash(g.file),field:g.key,records:g.records.length})),identity_conflicts:identityConflicts,total_line_count_conflicts:totalConflicts,required_coverage:requiredCoverage,partial_test_sources:partialTests,tests_executed:false,application_executed:false,all_original_leads_disposed:true}};
const diagnostic={area:"tui",entries:spec.entries.length,ledger:ledger.length,surfaces:surfaces.length,candidates:candidate_accounting,source_records:normalized.length,source_visible_lines:normalized.reduce((n,r)=>n+r.visible_lines,0),fully_read_files:normalized.filter(r=>r.full_file_read).length,excluded_source_files:normalized.filter(r=>r.blocked_ranges.length).map(r=>({path:r.path,redacted:r.excluded_redacted_lines,unverified:r.excluded_unverified_lines})),identity_conflicts:identityConflicts,total_line_count_conflicts:totalConflicts,unresolved_required_source:unresolvedRequired,partial_tests:partialTests,claims:claims.length,supplemental_ledger:supplemental_ledger.length};
if(mode==="preview"){console.log(JSON.stringify(diagnostic));process.exit(0);}
if(identityConflicts.length||unresolvedRequired.length||partialTests.some(t=>t.ordinary_missing_ranges.length))throw Error("Coverage gate failed "+JSON.stringify(diagnostic));
const write=(n,v)=>{const f=path.join(base,n),tmp=f+".tui-closeout-tmp";fs.writeFileSync(tmp,JSON.stringify(v,null,2)+"\n");fs.renameSync(tmp,f);return{path:"docs/ice-gap/evidence/"+n,sha256:hash(n),bytes:fs.statSync(f).size};};
const outputs=[write("proposal-tui.json",proposal),write("tui-source-review.json",finalReview),write("tui-closeout-coverage.json",diagnostic)];
console.log(JSON.stringify({outputs,...diagnostic}));
