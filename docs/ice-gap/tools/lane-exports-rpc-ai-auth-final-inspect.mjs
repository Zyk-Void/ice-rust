import { auditHelperEvidence } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import fs from "node:fs";
import crypto from "node:crypto";
const base=auditHelperEvidence+'/';
const read=(name)=>JSON.parse(fs.readFileSync(base+name,"utf8"));
const hash=(name)=>crypto.createHash("sha256").update(fs.readFileSync(base+name)).digest("hex");
const mode=process.argv[2];
if(mode==="validate"){
 const qa=read("qa-unnamed-exports-2026-10-20.json");
 const summary=read("lane-exports-rpc-ai-auth-read-summary.json");
 const areas=["ai","extensions","providers-auth","rpc-remote"];
 const errors=[];const per_area=[];
 const key=(d)=>JSON.stringify([d.symbol,d.kind,d.ts]);
 const count=(xs)=>{const m=new Map();for(const x of xs)m.set(key(x),(m.get(key(x))??0)+1);return m;};
 for(const area of areas){
  const name="lane-exports-rpc-ai-auth-"+area+".json";const p=read(name);
  const q=qa.symbols[area],pm=count(p.dispositions),qm=count(q);
  for(const [k,n] of qm)if(pm.get(k)!==n)errors.push(area+" QA mismatch "+k);
  for(const [k,n] of pm)if(qm.get(k)!==n)errors.push(area+" extra/duplicate "+k);
  const updates=new Map(p.entry_updates.map(x=>[x.id,x]));
  for(const d of p.dispositions){
   const original=q.find(x=>key(x)===key(d));
   if(original && JSON.stringify(original.covering_entries)!==JSON.stringify(d.covering_entries))errors.push(area+" covering metadata changed "+key(d));
   const match=/^ice:(.*):(\d+)$/.exec(d.ts);const sr=d.source_review;
   if(!match||sr.path!==match[1]||+match[2]<sr.reviewed_range[0]||+match[2]>sr.reviewed_range[1]||!/^[0-9a-f]{64}$/.test(sr.sha256))errors.push(area+" invalid source range/hash "+key(d));
   if(!summary.read_reviews.some(r=>r.area===area&&r.path===sr.path&&r.sha256===sr.sha256&&r.artifact===sr.artifact&&r.reviewed_ranges.some(([a,b])=>a<=sr.reviewed_range[0]&&b>=sr.reviewed_range[1])))errors.push(area+" source review missing from summary "+key(d));
   if(!d.reason||!d.disposition||!d.entry_ids?.length)errors.push(area+" incomplete disposition "+key(d));
   for(const id of d.entry_ids){const u=updates.get(id);if(!u)errors.push(area+" missing entry update "+id);else if(!u.source_append.includes("\x60"+d.symbol+"\x60")||!u.source_append.includes(d.ts))errors.push(area+" missing exact source name/ref "+key(d)+" "+id);}
  }
  const runtime=p.dispositions.filter(d=>d.kind==="runtime").length,type=p.dispositions.filter(d=>d.kind==="type").length;
  if(runtime!==qa.summary[area].runtime||type!==qa.summary[area].type||p.totals.records!==p.dispositions.length||p.new_ids.length)errors.push(area+" totals/new IDs mismatch");
  per_area.push({area,records:p.dispositions.length,runtime,type,entry_updates:p.entry_updates.length,proposal:name,sha256:hash(name)});
 }
 const result={status:errors.length?"failed":"passed",generated_at:"2026-10-20",reviewer:"/root/exports_rpc_ai_auth",scope:"Saved proposal structure and correspondence against the original QA records and lane read summary. This is evidence validation, not execution of application code or tests.",totals:per_area.reduce((a,r)=>({records:a.records+r.records,runtime:a.runtime+r.runtime,type:a.type+r.type,entry_updates:a.entry_updates+r.entry_updates}),{records:0,runtime:0,type:0,entry_updates:0}),per_area,redaction_limits:summary.redactions,checks:["Exact multiset of original symbol/kind/TypeScript-source tuples is preserved, including distinct overload lines.","Each record has a disposition, rationale, reviewed source range, SHA-256, and matching lane read-summary row.","Each selected existing entry includes the exact symbol name and source reference in its additive source field.","Original QA covering-entry metadata is preserved without treating it as authoritative ownership.","Per-area runtime/type counts match QA; no new entry IDs."],errors};
 console.log(JSON.stringify(result,null,2));
 if(errors.length)process.exitCode=1;
}else if(mode==="hunks-index"){
 const name="active-checkout-diff-hunks-complete.json",map=read(name);
 console.log(JSON.stringify({artifact:name,sha256:hash(name),keys:Object.keys(map),shape:Object.fromEntries(Object.entries(map).map(([k,v])=>[k,Array.isArray(v)?{type:"array",length:v.length,first:v[0]}:typeof v==="object"&&v!==null?{type:"object",keys:Object.keys(v).slice(0,20)}:v]))},null,2));
}else if(mode==="hunk-files"||mode==="hunk-batch"){
 const name="active-checkout-diff-hunks-complete.json",map=read(name);
 const rows=map.rows.filter(r=>{
  const p=r.old_path??r.active_path;
  const ownedCli=["config.rs","settings.rs","session.rs","resource_dirs.rs","updates.rs","prebuilt_release.rs","changelog.rs","brand.rs"].map(f=>"crates/ice-cli/src/"+f);
  return /^crates\/ice-(ai|agent|tools|extensions|plugin-sdk)\//.test(p)||p==="crates/ice-cli/Cargo.toml"||p.startsWith("crates/ice-cli/tests/")||(p.startsWith("crates/ice-cli/src/")&&!ownedCli.includes(p));
 }).sort((a,b)=>a.old_path.localeCompare(b.old_path));
 if(mode==="hunk-files"){
  console.log(JSON.stringify({artifact:name,sha256:hash(name),identities:map.identities,rows:rows.map((r,index)=>({index,old_path:r.old_path,active_path:r.active_path,old_sha256:r.old.sha256,active_sha256:r.active.sha256,diff_sha256:r.diff_sha256,changes:r.changes.length,unified_hunks:r.unified_diff.split("\n").filter(x=>x.startsWith("@@ ")).length,diff_bytes:Buffer.byteLength(r.unified_diff)}))},null,2));
 }else{
  const start=Number(process.argv[3]??0),count=Number(process.argv[4]??1);
  for(const [i,r] of rows.slice(start,start+count).entries()){
   console.log(JSON.stringify({index:start+i,old_path:r.old_path,active_path:r.active_path,old_sha256:r.old.sha256,active_sha256:r.active.sha256,diff_sha256:r.diff_sha256,changes:r.changes}));
   console.log(r.unified_diff);
  }
 }
}else if(mode==="validate-native"){
 const mapName="active-checkout-diff-hunks-complete.json",reviewName="lane-exports-rpc-ai-auth-native-hunk-review.json";
 const map=read(mapName),review=read(reviewName),errors=[];
 const ownedCli=["config.rs","settings.rs","session.rs","resource_dirs.rs","updates.rs","prebuilt_release.rs","changelog.rs","brand.rs"].map(f=>"crates/ice-cli/src/"+f);
 const expected=map.rows.filter(r=>{const p=r.old_path??r.active_path;return /^crates\/ice-(ai|agent|tools|extensions|plugin-sdk)\//.test(p)||p==="crates/ice-cli/Cargo.toml"||p.startsWith("crates/ice-cli/tests/")||(p.startsWith("crates/ice-cli/src/")&&!ownedCli.includes(p));});
 const actual=new Map(review.files.map(f=>[f.old_path,f]));
 if(review.source_map.sha256!==hash(mapName))errors.push("source map hash drift");
 if(actual.size!==review.files.length||actual.size!==expected.length)errors.push("file count mismatch/duplicate");
 let hunks=0,changes=0,redactions=0;
 for(const r of expected){
  const f=actual.get(r.old_path);if(!f){errors.push("missing file "+r.old_path);continue;}
  if(f.active_path!==r.active_path||f.old_sha256!==r.old.sha256||f.active_sha256!==r.active.sha256||f.captured_diff_sha256!==r.diff_sha256)errors.push("identity mismatch "+r.old_path);
  const parsed=[];let h=null;
  for(const line of r.unified_diff.split("\n")){
   const m=/^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/.exec(line);
   if(m){h={header:line,old_start:+m[1],old_count:m[2]===undefined?1:+m[2],active_start:+m[3],active_count:m[4]===undefined?1:+m[4],lines:[]};parsed.push(h);}
   else if(h&&/^[ +-]|^\\ No newline/.test(line))h.lines.push(line);
  }
  if(parsed.length!==f.hunks.length)errors.push("hunk count mismatch "+r.old_path);
  for(let i=0;i<parsed.length;i++){
   const h=parsed[i],v=f.hunks[i];if(!v){errors.push("missing hunk "+r.old_path+":"+h.old_start);continue;}
   for(const k of ["old_start","old_count","active_start","active_count"])if(h[k]!==v[k])errors.push(k+" mismatch "+r.old_path+":"+h.old_start);
   if(v.unified_header!==h.header||JSON.stringify(v.diff_lines)!==JSON.stringify(h.lines))errors.push("hunk text mismatch "+r.old_path+":"+h.old_start);
   if(JSON.stringify(v.old_range)!==JSON.stringify([h.old_start,h.old_start+Math.max(0,h.old_count-1)])||JSON.stringify(v.active_range)!==JSON.stringify([h.active_start,h.active_start+Math.max(0,h.active_count-1)]))errors.push("range mismatch "+r.old_path+":"+h.old_start);
   if(!["identity-only","behavioral","formatting"].includes(v.classification)||!v.confirmation||!v.review_status)errors.push("missing review "+r.old_path+":"+h.old_start);
   const oldCount=h.lines.filter(l=>l[0]===" "||l[0]==="-").length,newCount=h.lines.filter(l=>l[0]===" "||l[0]==="+").length;
   if(oldCount!==h.old_count||newCount!==h.active_count)errors.push("diff body count mismatch "+r.old_path+":"+h.old_start);
   redactions+=h.lines.filter(l=>/REDACTED|SECRET_REDACT/i.test(l)).length;
  }
  if(JSON.stringify(f.hunks.flatMap(h=>h.changed_ranges))!==JSON.stringify(r.changes))errors.push("exact change block mismatch "+r.old_path);
  hunks+=parsed.length;changes+=r.changes.length;
 }
 const result={status:errors.length?"failed":"passed",reviewer:"/root/exports_rpc_ai_auth",review_date:"2026-10-20",artifact:"evidence/"+reviewName,artifact_sha256:hash(reviewName),source_map:"evidence/"+mapName,source_map_sha256:hash(mapName),totals:{files:expected.length,hunks,exact_change_blocks:changes,redaction_markers:redactions},checks:["Exact assigned changed-file set matches the captured map.","Old/active paths, full-file SHA-256 values, and captured diff hashes match.","Every hunk has exact old/active ranges, body text, classification, and confirmation.","Hunk body old/new line counts match each unified header; every exact change block is covered once.","This validates saved review evidence; no application/build/test execution occurred."],errors};
 console.log(JSON.stringify(result,null,2));if(errors.length)process.exitCode=1;
}else{
 throw new Error("unknown mode");
}
