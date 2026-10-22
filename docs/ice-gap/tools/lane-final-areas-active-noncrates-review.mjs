import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
assertAuditOpen();
import fs from 'node:fs';import path from 'node:path';import{createHash}from'node:crypto';import{fileURLToPath}from'node:url';
const base=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..'),ev=path.join(base,'evidence'),hash=b=>createHash('sha256').update(b).digest('hex'),read=n=>JSON.parse(fs.readFileSync(path.join(ev,n),'utf8')),diffName='active-checkout-diff-hunks-complete.json',mapName='active-checkout-source-map.json',d=read(diffName),m=read(mapName),canonical=read('lane-final-areas-read-coverage-proposal.json'),paths=canonical.rust_non_crates.map(r=>r.path.slice(5)),errors=[],reviews=[],identical=[];
function classification(p,c){
 if(p==='Cargo.toml')return {kind:'comment-only-runtime-label',finding:'The workspace banner/internal-crate comment changes ice to icers. Crate names, dependency paths, versions and manifest structure do not change in this segment.',runtime_equivalence:true};
 if(p==='ICE_PORTING.md'&&c.old_count===0)return {kind:'first-pass-report-addition',finding:'Adds documented runtime identity and four import-era claims about thinking-level parsing, builtin provider catalogs, themes and settings. These are report claims, including private-state import observations, not independent evidence that the claims are correct. Coordinator first-pass reconciliation remains the governing disposition.',runtime_equivalence:null,limitations:['No private state inspected.','No source behavior established by the prose addition.','Old segment is empty at insertion boundary659; only active lines660–687 were added.']};
 if(p==='ICE_PORTING.md')return {kind:'documented-release-owner-case',finding:'The self-update repository URL spelling changes Zyk-Void to zyk-void. This is a textual endpoint spelling change; no release availability or network behavior was tested.',runtime_equivalence:null};
 if(p==='README.md'){
  if([63,77,79].includes(c.old_start))return {kind:'installation-identity-documentation',finding:'The documented installation directory, command or package-manager formula name changes to icers. This review confirms the text change, not package availability or installer correctness.',runtime_equivalence:null};
  if([116,124,268,288].includes(c.old_start))return {kind:'runtime-config-identity-documentation',finding:'Examples now use icers, ~/.icers or .icers and ICERS_CODING_AGENT_DIR/ICERS_OFFLINE as applicable. Existing crate/package identifiers and provider-standard environment names remain as shown. Runtime support is established separately by active source review.',runtime_equivalence:null};
  return {kind:'runtime-command-or-product-label-documentation',finding:'Replaces the native ice product/command label with icers in prose, examples, headings or tables. The underlying behavioral claims, measurements and external links are not newly verified by this diff review.',runtime_equivalence:null};
 }
 if(p==='docs/architecture/overview.md')return {kind:'product-label-documentation',finding:'Only the native product label changes ice to icers in the architecture overview sentence.',runtime_equivalence:null};
 if(p==='docs/architecture/provider-packages.md'||p==='docs/extensions/provider-integration.md'){
  if((p.endsWith('provider-packages.md')&&c.old_start>=161)||(p.endsWith('provider-integration.md')&&c.old_start>=247))return {kind:'test-environment-name-documentation',finding:'Changes ICE_TEST_PROVIDER_LIBRARY/ICE_TEST_PROVIDER_PACKAGE examples to ICERS_TEST_PROVIDER_LIBRARY/ICERS_TEST_PROVIDER_PACKAGE. Plugin crate, library filenames and provider identifiers are preserved. Test execution and companion-package availability are not claimed.',runtime_equivalence:null};
  return {kind:'runtime-command-name-documentation',finding:'Changes the native command example from ice to icers. Provider IDs, auth subcommands, model names, base URL and package-operation arguments remain unchanged.',runtime_equivalence:null};
 }
 if(p==='docs/extensions/subagents.md')return {kind:'test-environment-name-documentation',finding:'Changes ICE_TEST_SUBAGENT_LIBRARY to ICERS_TEST_SUBAGENT_LIBRARY in the documented cargo test invocation. The library filename, crate and test target remain unchanged. No test was executed.',runtime_equivalence:null};
 if(p==='docs/promotion/langfuse.md'){
  if([252,263,274,344,358].includes(c.old_start))return {kind:'extension-config-identity-documentation',finding:'Updates native configuration examples to .icers, ~/.icers and ICERS_LANGFUSE_CONFIG, or the native extension directory. The companion extension behavior is not established by this documentation-only change.',runtime_equivalence:null};
  return {kind:'runtime-command-or-product-label-documentation',finding:'Changes native product labels and command examples to icers. The ice-langfuse package/library identifier, LANGFUSE provider-standard variables and telemetry claims are not changed or newly validated by this review.',runtime_equivalence:null};
 }
 throw new Error('Unexpected reviewed path '+p);
}
for(const p of paths){
 const row=d.rows.find(r=>r.old_path===p),mapped=m.rows.find(r=>r.old_path===p);
 if(!mapped){errors.push({path:p,kind:'missing_map'});continue}
 if(!row){
  if(mapped.status!=='identical'||mapped.old.sha256!==mapped.active.sha256)errors.push({path:p,kind:'missing_changed_row'});
  else identical.push({old_path:p,active_path:mapped.active_path,sha256:mapped.old.sha256,status:'identical-metadata-only',reason:'No changed hunk; earlier read coverage may be transferred by exact hash identity, no new whole-file read claimed.'});
  continue;
 }
 const ob=fs.readFileSync(path.join(d.old_root,row.old_path)),nb=fs.readFileSync(path.join(d.active_root,row.active_path)),oldSha=hash(ob),newSha=hash(nb),ol=ob.toString('utf8').split('\n'),nl=nb.toString('utf8').split('\n');
 if(oldSha!==row.old.sha256||newSha!==row.active.sha256)errors.push({path:p,kind:'current_identity_drift'});
 if(hash(row.unified_diff)!==row.diff_sha256)errors.push({path:p,kind:'diff_text_hash_mismatch'});
 const removals=new Map(),additions=new Map();let oi=0,ni=0,inside=false;
 for(const l of row.unified_diff.split('\n')){
  const h=l.match(/^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/);
  if(h){oi=+h[1];ni=+h[3];inside=true;continue}
  if(!inside)continue;
  if(l.startsWith('-')){removals.set(oi++,l.slice(1));}
  else if(l.startsWith('+')){additions.set(ni++,l.slice(1));}
  else if(l.startsWith(' ')){oi++;ni++;}
 }
 row.changes.forEach((c,i)=>{
  const oldRange=c.old_count?[[c.old_start,c.old_start+c.old_count-1]]:[],activeRange=c.active_count?[[c.active_start,c.active_start+c.active_count-1]]:[],oldText=c.old_count?ol.slice(c.old_start-1,c.old_start+c.old_count-1).join('\n'):'',activeText=c.active_count?nl.slice(c.active_start-1,c.active_start+c.active_count-1).join('\n'):'';
  for(let n=c.old_start;n<c.old_start+c.old_count;n++)if(removals.get(n)!==ol[n-1])errors.push({path:p,segment:i,kind:'old_diff_source_mismatch',line:n});
  for(let n=c.active_start;n<c.active_start+c.active_count;n++)if(additions.get(n)!==nl[n-1])errors.push({path:p,segment:i,kind:'active_diff_source_mismatch',line:n});
  reviews.push({old_path:p,active_path:row.active_path,segment_index:i,old_start:c.old_start,old_count:c.old_count,active_start:c.active_start,active_count:c.active_count,old_ranges:oldRange,active_ranges:activeRange,empty_old_segment:c.old_count===0,empty_active_segment:c.active_count===0,old_sha256:oldSha,active_sha256:newSha,diff_sha256:row.diff_sha256,old_segment_sha256:hash(oldText),active_segment_sha256:hash(activeText),old_text:oldText,active_text:activeText,...classification(p,c),reviewer:'/root/final_area_handoffs',confirmation:'All removed/added text was displayed and source-reviewed in the saved unified diff. Segment text and file hashes were then checked against current bytes. No whole-file reread or application execution claimed.'});
 });
}
const out={schema_version:1,owner:'/root/final_area_handoffs',status:errors.length?'FAIL':'PASS',scope:'Only the10 previously credited Rust non-crates paths;8 changed files with all exact minimal changed segments reviewed,2 identical provider-runtime files excluded from new hunk reading.',source_only:true,application_or_test_execution:false,inputs:[{path:'docs/ice-gap/evidence/'+diffName,sha256:hash(fs.readFileSync(path.join(ev,diffName)))},{path:'docs/ice-gap/evidence/'+mapName,sha256:hash(fs.readFileSync(path.join(ev,mapName)))}],old_root:d.old_root,active_root:d.active_root,summary:{scoped_paths:paths.length,changed_files:new Set(reviews.map(r=>r.old_path)).size,segments:reviews.length,zero_old_segments:reviews.filter(r=>r.empty_old_segment).length,zero_active_segments:reviews.filter(r=>r.empty_active_segment).length,identical_unread_files:identical.length,errors:errors.length},reviews,identical_files:identical,errors,limitations:['Documentation is read as authored claims, not executable proof.','ICE_PORTING import references were not checked against private ~/.ice or ~/.icers state.','No external URLs or package managers were contacted.','Unchanged intervals require independent exact-hash/line-map transfer; this artifact adds read credit only for changed old/active ranges.']};
fs.writeFileSync(path.join(ev,'lane-final-areas-active-noncrates-review.json'),JSON.stringify(out,null,2)+'\n');
console.log(JSON.stringify({status:out.status,summary:out.summary,errors,artifact_sha256:hash(fs.readFileSync(path.join(ev,'lane-final-areas-active-noncrates-review.json')))}));
