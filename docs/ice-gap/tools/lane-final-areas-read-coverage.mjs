import { auditHelperEvidence } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
assertAuditOpen();
import fs from 'node:fs';
import path from 'node:path';
import {createHash} from 'node:crypto';
import {fileURLToPath} from 'node:url';
const root='/home/mewtwo/Zks/ice-rust', ev=auditHelperEvidence;
const roots={ice:'/home/mewtwo/Zks/ice',rust:root};
const areas=['ai','agent','tools','session','settings','extensions','rpc-remote','tui','subagents','safe-verify','observability','cognee','blackhole','providers-auth','cli-packaging','misc'];
const hash=b=>createHash('sha256').update(b).digest('hex'), cache=new Map(), inputs=new Map(), claims=new Map(), blocks=new Map(), skipped=[], malformed=[];
function read(n){if(!cache.has(n)){const b=fs.readFileSync(path.join(ev,n));cache.set(n,JSON.parse(b.toString('utf8')));inputs.set(n,{path:'docs/ice-gap/evidence/'+n,sha256:hash(b),bytes:b.length,roles:[]});}return cache.get(n)}
function use(n,role){const o=read(n),d=inputs.get(n);if(!d.roles.includes(role))d.roles.push(role);return o}
function normal(p,side='ice'){
 if(typeof p!=='string')return null;
 if(p.startsWith('/home/mewtwo/Zks/ice-rust/'))p='rust:'+p.slice('/home/mewtwo/Zks/ice-rust/'.length);
 else if(p.startsWith('/home/mewtwo/Zks/ice/'))p='ice:'+p.slice('/home/mewtwo/Zks/ice/'.length);
 else if(p.startsWith('/'))return null;
 if(!/^(ice|rust):/.test(p))p=((p.startsWith('crates/')||['rust','native'].includes(side))?'rust':'ice')+':'+p;
 const [s,...rest]=p.split(':'),rel=rest.join(':');
 if(!['ice','rust'].includes(s)||!rel||rel.startsWith('/')||rel.split('/').includes('..')||/(^|\/)\.(?:ice|icers)(\/|$)/.test(rel))return null;
 return s+':'+rel;
}
function ranges(v){if(!Array.isArray(v))return [];if(v.length===2&&v.every(Number.isInteger))return v[0]>0&&v[1]>=v[0]?[[v[0],v[1]]]:[];return v.flatMap(x=>Array.isArray(x)&&x.length===2&&x.every(Number.isInteger)&&x[0]>0&&x[1]>=x[0]?[x]:[])}
function union(xs){const out=[];for(const[a,b]of xs.slice().sort((x,y)=>x[0]-y[0]||x[1]-y[1])){const q=out.at(-1);if(q&&a<=q[1]+1)q[1]=Math.max(q[1],b);else out.push([a,b]);}return out}
function lines(v){if(Number.isInteger(v)&&v>0)return[v];if(!Array.isArray(v))return[];const out=[];for(const x of v){if(Number.isInteger(x)&&x>0)out.push(x);else if(Array.isArray(x)&&x.length===2&&x.every(Number.isInteger)&&x[0]>0&&x[1]>=x[0]&&x[1]-x[0]<50000)for(let i=x[0];i<=x[1];i++)out.push(i);}return out}
function recordRanges(r){return ranges(r.visible_ranges||r.reviewed_ranges||r.ranges||r.read_ranges||r.range||r.reviewed_range||(Number.isInteger(r.startLine)?[r.startLine,r.endLine]:Number.isInteger(r.start_line)?[r.start_line,r.end_line]:Number.isInteger(r.start)?[r.start,r.end]:[]));}
function addBlock(p,kind,ns,art,loc,reason){
 p=normal(p);const ls=lines(ns);if(!p||!ls.length)return;
 if(!blocks.has(p))blocks.set(p,[]);
 blocks.get(p).push({kind,lines:[...new Set(ls)].sort((a,b)=>a-b),artifact:art,record:loc,reason});
}
function addClaim(n,loc,r,side,owner,legacy=false){
 if(!r||typeof r!=='object')return;
 const p=normal(r.path||r.file, r.side||r.repo||r.repository||side||'ice'),rs=recordRanges(r);
 if(r.metadata_only===true){skipped.push({artifact:n,record:loc,path:p,reason:'Explicit metadata_only record adds no read credit'});return}
 if(!p||!rs.length){skipped.push({artifact:n,record:loc,path:r.path||r.file||null,reason:!p?'Outside captured source roots or no source path':'No explicit reviewed ranges; total line count is metadata only'});return}
 if(!claims.has(p))claims.set(p,[]);
 const sh=typeof r.sha256==='string'&&/^[a-f0-9]{64}$/.test(r.sha256)?r.sha256:null;
 claims.get(p).push({artifact:n,artifact_sha256:inputs.get(n).sha256,record:loc,reader:r.owner||r.reader||r.read_by||r.owners||owner||'Attributed to original source artifact; no new independent review claimed',ranges:rs,sha256:sh,legacy,derivation:r.derivation||null,notes:r.notes||r.review||r.evidence||null});
}
function field(n,k,side='ice'){
 const o=use(n,'explicit_read_claims'),v=k?k.split('.').reduce((a,b)=>a?.[b],o):o;
 if(!Array.isArray(v)){malformed.push({artifact:n,field:k,reason:'Expected an explicit read array'});return}
 v.forEach((r,i)=>addClaim(n,(k||'root')+'['+i+']',r,side,o.owner||o.reviewer||o.worker||o.area));
}
const legacy=use('read-coverage.json','legacy_canonical'),legacyFiles=legacy.files||legacy;
for(const[p,r]of Object.entries(legacyFiles))if(/^(ice|rust):/.test(p))addClaim('read-coverage.json',p,{path:p,...r},null,'Legacy canonical record; original author not reconstructed',true);
const acceptedPaths=new Set();
for(const a of areas){const n=a+'.json',o=use(n,'accepted_area_manifest');for(const[r,i]of(o.read_evidence||[]).map((r,i)=>[r,i])){addClaim(n,'read_evidence['+i+']',r,null,a);const p=normal(r.path,r.repo);if(p)acceptedPaths.add(p)}}
const selectors={
 'ai-source-review.json':['read_evidence'],
 'ai-combined-read-manifest.json':[''],
 'ai-schema-source-review.json':['files'],
 'auth-normalized-read-manifest.json':['read_evidence','independent_coordinator_reads'],
 'cli-combined-read-manifest.json':['read_records'],
 'cli-source-review.json':['read_ranges','shared_tui_tests','reused_test_reads','merged_reads'],
 'cli-packaging-supplement-source-review.json':['actual_read_ranges','reused_read_evidence'],
 'extensions-source-review.json':['read_evidence'],
 'tools-read-evidence.json':['read_evidence'],
 'session-source-review.json':['files'],
 'settings-read-progress.json':[''],
 'subagents-read-evidence.json':['read_evidence'],
 'observability-read-progress.json':['read_evidence'],
 'rpc-source-review.json':['reads','test_paths','merged_reads'],
 'tui-source-review.json':['read_evidence'],
 'tui-terminal-source-review.json':['read_records'],
 'tui-closeout-source-review.json':['read_evidence'],
 'tui-closeout-final-source-supplement.json':['read_evidence','shared_read_evidence'],
 'tui-native-controller-supplement.json':['source_reads'],
 'tui-native-controller-test-supplement.json':['source_reads'],
 'tui-native-helper-supplement.json':['source_reads'],
 'misc-source-review.json':['read_ranges','reused_source_reads'],
 'misc-claude-sdk-source-review.json':['read_evidence'],
 'misc-evals-source-review.json':['files'],
 'misc-coordinator-docs-review.json':['read_evidence'],
 'provider-wire-supplement.json':['read_records','full_read_manifest'],
 'coordinator-additional-tui-eval-review.json':['read_evidence'],
 'coordinator-ai-schema-review.json':['reads'],
 'coordinator-export-provider-form-review.json':['read_evidence'],
 'coordinator-native-command-review.json':['read_evidence'],
 'coordinator-native-defect-review.json':['read_evidence'],
 'coordinator-queue-abort-review.json':['read_evidence'],
 'qa-ai-test-closure.json':['ts_test_reads','rust_integration_test_reads','generated_data_groups'],
 'qa-auth-test-closure.json':['tests'],
 'qa-cli-test-closure.json':['tests'],
 'qa-rpc-test-closure.json':['ts_tests','native_test_reads'],
 'qa-session-test-closure.json':['files'],
 'qa-settings-test-closure.json':['read_evidence'],
 'qa-tools-coordinator.json':['read_evidence'],
 'qa-coordinator-subagents.json':['independent_source_reads'],
 'qa-extensions-coordinator.json':['read_evidence'],
 'qa-observability-independent.json':['read_evidence'],
 'qa-auth-coordinator-review.json':['source_reads'],
 'qa-rpc-coordinator-review.json':['source_reads'],
 'qa-cli-coordinator.json':['source_reads'],
 'lane-root-source-review.json':['records'],
 'lane-root-declarations-agent-cli-packaging-settings-observability.json':['records'],
 'lane-root-declarations-tools.json':['records'],
 'lane-exports-rpc-ai-auth-read-summary.json':['read_reviews','extra_reviews'],
 'lane-exports-sub-session-source-review-corrected.json':['reads','native_reads'],
 'lane-exports-sub-session-subagents-source-review.json':['files','additional_implementation_reads'],
 'lane-final-areas-source-supplement.json':['read_evidence'],
 'lane-final-areas-export-type-contracts.json':['records'],
};
for(const[n,ks]of Object.entries(selectors))for(const k of ks)field(n,k);
for(const n of fs.readdirSync(ev).filter(n=>/^lane-final-areas-export-read-(misc|tui)-\d+\.json$/.test(n))){
 const o=use(n,'explicit_export_snippets');o.records.forEach((r,i)=>addClaim(n,'records['+i+'].source',r.source,'ice',o.owner));
}
for(const n of fs.readdirSync(ev).filter(n=>/^lane-final-areas-export-ranges-.*\.json$/.test(n)))field(n,'files');
for(const n of ['lane-final-areas-export-extra-reads-1.json','lane-final-areas-export-extra-reads-2.json'])field(n,'files');

const copyProofs=[],copyIdentityErrors=[];
for(const r of use('blackhole-copies.json','byte_identical_copy_proof')){
 if(r.same_bytes!==true)continue;
 const source=normal(r.source),copy=normal(r.copy),sourceClaims=claims.get(source);
 if(!source||!copy||!sourceClaims?.length){copyIdentityErrors.push({source,copy,reason:'Copy lacks previously reviewed source'});continue}
 const sourceBytes=fs.readFileSync(path.join(roots.ice,r.source)),copyBytes=fs.readFileSync(path.join(roots.ice,r.copy)),sh=hash(sourceBytes),ch=hash(copyBytes);
 if(sh!==ch||sh!==r.source_sha256||ch!==r.copy_sha256){copyIdentityErrors.push({source,copy,source_sha256:sh,copy_sha256:ch,reason:'Current bytes do not match certified identity proof'});continue}
 const derivedRanges=union(sourceClaims.filter(x=>!x.sha256||x.sha256===sh).flatMap(x=>x.ranges));
 const proof={source,copy,source_sha256:sh,copy_sha256:ch,source_ranges:derivedRanges,proof_artifact:'blackhole-copies.json',proof_artifact_sha256:inputs.get('blackhole-copies.json').sha256,method:'Current source and copy bytes hash identically to recorded proof; exact reviewed content is reused. Copy was not independently source-read.'};
 copyProofs.push(proof);
 addClaim('blackhole-copies.json','copy='+r.copy,{path:copy,ranges:derivedRanges,sha256:ch,derivation:proof,notes:'Derived from byte-identical reviewed example; independent copy read not claimed'},'ice','Original Blackhole source reader and recorded identical-copy proof');
}

// These are evidence records, not new source reads. Scan only explicit exclusion fields and redacted text markers.
// Raw superseded session read claims never enter coverage; their explicit redaction metadata remains conservative.
const exclusionNames=fs.readdirSync(ev).filter(n=>n.endsWith('.json')&&(
 areas.includes(n.slice(0,-5))||/source-review|read-manifest|test-closure|redaction-markers|coordinator.*review|native.*supplement|closeout.*supplement/.test(n)||
 /^(lane-root-(source-review|declarations)|lane-exports-.*(read|source-review)|lane-final-areas-(source-supplement|export-(read|ranges|extra|type)))/.test(n)
)&&!n.startsWith('lane-final-areas-read-coverage'));
function scanExclusions(x,n,loc,parent,context=''){
 if(!x||typeof x!=='object')return;
 if(Array.isArray(x)){x.forEach((v,i)=>scanExclusions(v,n,loc+'['+i+']',parent,context));return}
 const p=normal(x.path||x.file,x.repo||x.side||x.repository)||parent;
 for(const[k,v]of Object.entries(x)){
  if(['excluded_redacted_lines','redacted_lines','redacted_exclusions','strict_visible_exclusions'].includes(k)&&Array.isArray(v)&&v.every(z=>typeof z==='number'||Array.isArray(z)))addBlock(p,'redacted',v,n,loc+'.'+k,'Explicit connector-redaction exclusion');
  if(['excluded_unverified_lines','blocked_ranges'].includes(k))addBlock(p,'unverified',v,n,loc+'.'+k,'Explicit source-visibility exclusion');
  if(k==='excluded_lines')addBlock(p,'unverified',v,n,loc+'.'+k,'Explicit excluded source lines; detailed parent record retains classification');
 }
 if(/redact/i.test(context)){
  addBlock(p,'redacted',x.line||x.lines||x.excluded_lines,n,loc,'Explicit redaction record');
 }
 if(context==='unverified_source_gaps')addBlock(p,'unverified',x.lines||x.ranges,n,loc,x.reason||'Explicit unverified source gap');
 // Direct snippets have visible source line numbers. Markers alone never imply unread intervening lines were read.
 for(const k of ['source','text','excerpt','declaration','visible']){
  const v=x[k];if(typeof v!=='string'||!/\[REDACTED(?:_[A-Z]+)?\]/.test(v))continue;
  const first=x.startLine||x.start_line||x.start||recordRanges(x)[0]?.[0], contiguous=recordRanges(x).length<=1;
  const ns=[];v.split('\n').forEach((line,i)=>{if(!/\[REDACTED(?:_[A-Z]+)?\]/.test(line))return;const m=line.match(/^\s*(\d+)\s*[|:]/);if(m)ns.push(+m[1]);else if(Number.isInteger(first)&&contiguous)ns.push(first+i)});
  addBlock(p,'redacted',ns,n,loc+'.'+k,'Observed connector marker in saved source excerpt; hidden text not reconstructed');
 }
 for(const[k,v]of Object.entries(x)){
  if(typeof v!=='object'||!v)continue;
  if(k==='strict_visible_exclusions'&& !Array.isArray(v)){for(const[q,ns]of Object.entries(v))addBlock(normal(q),'redacted',ns,n,loc+'.'+k+'.'+q,'Explicit terminal-fragment redaction map');continue}
  const keyed=normal(k);
  scanExclusions(v,n,loc+'.'+k,keyed&&/^(ice|rust):/.test(k)?keyed:p,k);
 }
}
for(const n of exclusionNames)scanExclusions(use(n,'exclusion_metadata'),n,'root',null);
addBlock('rust:crates/ice-cli/src/interactive_tui/sessions.rs','unverified',[627],'tui-closeout-final-source-supplement.json','exclusion_policy.unverified_source_gaps','Conservative unresolved SESSION/TUI visibility conflict; no hidden text recovered');

for(const proof of copyProofs)for(const b of blocks.get(proof.source)||[])addBlock(proof.copy,b.kind,b.lines,b.artifact,b.record,'Byte-identical source exclusion propagated from '+proof.source+'; '+b.reason);

function subtract(xs,excluded){const out=[];for(const[a,b]of union(xs)){let start=a;for(const n of excluded){if(n<start)continue;if(n>b)break;if(n>start)out.push([start,n-1]);start=n+1}if(start<=b)out.push([start,b])}return out}
function count(xs){return xs.reduce((a,[s,e])=>a+e-s+1,0)}
const baseline=use('baseline.json','baseline_metadata'), frozen=use('resume-source-identities-2026-10-19T222055Z.json','baseline_hash_metadata');
const dirtyTests=baseline.ts.status.split('\n').map(row=>({path:row.slice(3),status:row.slice(0,2)})).filter(r=>(r.path.includes('/test/')||r.path.includes('/tests/'))&&r.status.trim());
const sourceErrors=[],identityConflicts=[],rangeTrims=[],files={},exclusionRows=[],baselineLimits=[];
for(const p of [...claims.keys()].sort()){
 const side=p.slice(0,p.indexOf(':')),rel=p.slice(p.indexOf(':')+1),sources=claims.get(p);let data;
 try{data=fs.readFileSync(path.join(roots[side],rel))}catch(e){sourceErrors.push({path:p,error:e.code});continue}
 const current=hash(data),str=data.toString('utf8'),split=str.split('\n'),total=str.length===0?0:split.length-(str.endsWith('\n')?1:0);
 const hs=[...new Set(sources.map(r=>r.sha256).filter(Boolean))],matching=hs.includes(current),mismatched=sources.filter(r=>r.sha256&&r.sha256!==current);
 if(mismatched.length)identityConflicts.push({path:p,current_sha256:current,claims:mismatched.map(r=>({artifact:r.artifact,record:r.record,sha256:r.sha256,ranges:r.ranges}))});
 const eligible=sources.filter(r=>!r.sha256||r.sha256===current),rawRanges=union(eligible.flatMap(r=>r.ranges)),b=blocks.get(p)||[];
 const redacted=[...new Set(b.filter(x=>x.kind==='redacted').flatMap(x=>x.lines))].sort((a,b)=>a-b);
 const redactedSet=new Set(redacted),unverified=[...new Set(b.filter(x=>x.kind==='unverified').flatMap(x=>x.lines))].filter(n=>!redactedSet.has(n)).sort((a,b)=>a-b);
 const allExcluded=[...redacted,...unverified].sort((a,b)=>a-b);
 for(const r of rawRanges)if(r[1]>total)rangeTrims.push({path:p,range:r,total_lines:total,only_terminal_empty:r[1]===total+1&&str.endsWith('\n')});
 const clipped=rawRanges.filter(([a])=>a<=total).map(([a,z])=>[a,Math.min(z,total)]),credited=subtract(clipped,allExcluded);
 const ordinaryMissing=subtract(total?[[1,total]]:[],[...allExcluded,...credited.flatMap(([a,z])=>Array.from({length:z-a+1},(_,i)=>a+i))].sort((a,b)=>a-b));
 const dirty=side==='ice'&&dirtyTests.find(r=>r.path===rel),initial=frozen.hashes?.[side]?.[rel]||null;
 if(dirty&&!initial)baselineLimits.push({path:p,baseline_status:dirty.status,baseline_worktree_sha256:null,current_read_sha256:matching?current:null,current_metadata_sha256:current,classification:'pre-existing-dirty-current-only',reason:'Original dirty worktree hash absent the captured source manifest; current read identity does not certify the historical baseline.',evidence:['baseline.json','resume-source-identities-2026-10-19T222055Z.json',...new Set(sources.map(r=>r.artifact))]});
 files[p]={ranges:credited,sha256:matching?current:null,current_sha256:current,total_lines:total,fully_read:ordinaryMissing.length===0&&allExcluded.length===0,visible_lines:count(credited),excluded_redacted_lines:redacted,excluded_unverified_lines:unverified,ordinary_missing_ranges:ordinaryMissing,legacy_canonical_record_preserved:!!legacyFiles[p],legacy_only_against_accepted_areas:!!legacyFiles[p]&&!acceptedPaths.has(p),source_only:true,tests_executed:false,read_credit_kind:sources.every(r=>r.derivation)?'derived-byte-identical-copy':'direct-or-inherited-source-review',read_sources:sources.map(r=>({...r,sha256_matches_current:r.sha256===null?null:r.sha256===current})),hash_scope:matching?'Canonical identity matches an existing read-evidence hash; current metadata check adds no source-read credit.':'Current metadata hash only; no read-time hash is asserted.',baseline_identity:dirty&&!initial?'pre-existing-dirty-current-only':'See original source-artifact attribution; no new baseline certification'};
 if(b.length)exclusionRows.push({path:p,excluded_redacted_lines:redacted,excluded_unverified_lines:unverified,sources:b});
}
const unmatchedExclusions=[...blocks.keys()].filter(p=>!files[p]).map(p=>({path:p,sources:blocks.get(p)}));
const summary={files:Object.keys(files).length,ice_files:Object.keys(files).filter(p=>p.startsWith('ice:')).length,rust_files:Object.keys(files).filter(p=>p.startsWith('rust:')).length,legacy_files:Object.keys(legacyFiles).filter(p=>/^(ice|rust):/.test(p)).length,legacy_retained:Object.values(files).filter(r=>r.legacy_canonical_record_preserved).length,accepted_unique_files:acceptedPaths.size,legacy_only_against_current_accepted:Object.values(files).filter(r=>r.legacy_only_against_accepted_areas).length,fully_visible_files:Object.values(files).filter(r=>r.fully_read).length,visible_lines:Object.values(files).reduce((a,r)=>a+r.visible_lines,0),files_with_exclusions:exclusionRows.length,redacted_lines:exclusionRows.reduce((a,r)=>a+r.excluded_redacted_lines.length,0),unverified_lines:exclusionRows.reduce((a,r)=>a+r.excluded_unverified_lines.length,0),canonical_missing_read_sha256:Object.values(files).filter(r=>!r.sha256).length,dirty_baseline_missing_hashes:baselineLimits.length,missing_sha256_claims:[...claims.values()].flat().filter(r=>!r.sha256).length,identity_conflict_files:identityConflicts.length,source_errors:sourceErrors.length,inputs:inputs.size,derived_copy_files:copyProofs.length,direct_or_inherited_read_files:Object.keys(files).length-copyProofs.length};

const requirementBytes=fs.readFileSync(path.join(root,'docs/ice-gap/requirements.md')),ledgerBytes=fs.readFileSync(path.join(root,'docs/ice-gap/ledger.csv'));
const generatedPath='ice:packages/ai/src/image-models.generated.ts';
const coveragePolicies=[{
 path:generatedPath,kind:'generated-data-group',not_read_ranges:files[generatedPath]?.ordinary_missing_ranges||[],direct_read_ranges:files[generatedPath]?.ranges||[],sha256:files[generatedPath]?.sha256,entry_ids:['AI-015','AI-057'],
 rule:{path:'docs/ice-gap/requirements.md',line:23,sha256:hash(requirementBytes),meaning:'Generated model data is excluded from exhaustive source reading; one ledger row describes what the generator produces.'},
 ledger:{path:'docs/ice-gap/ledger.csv',sha256:hash(ledgerBytes),row_path:'packages/ai/src/image-models.generated.ts',note:'Generated values grouped by fully read scripts/generate-image-models.ts and image-models.ts consumer; only 1-35 inspected directly. No individual model-value parity claim.'},
 generator:{path:'ice:packages/ai/scripts/generate-image-models.ts',ranges:files['ice:packages/ai/scripts/generate-image-models.ts']?.ranges,sha256:files['ice:packages/ai/scripts/generate-image-models.ts']?.sha256,fully_read:files['ice:packages/ai/scripts/generate-image-models.ts']?.fully_read},
 consumer:{path:'ice:packages/ai/src/image-models.ts',ranges:files['ice:packages/ai/src/image-models.ts']?.ranges,sha256:files['ice:packages/ai/src/image-models.ts']?.sha256,fully_read:files['ice:packages/ai/src/image-models.ts']?.fully_read},
 evidence:{path:'docs/ice-gap/evidence/qa-ai-test-closure.json',field:'generated_data_groups[0]',sha256:inputs.get('qa-ai-test-closure.json').sha256},
 limitation:'Generated value rows remain unread and are not relabeled as redacted/unverified or added to visible read coverage. Generator was source-read, not executed.'
}];

const output={schema_version:1,owner:'/root/final_area_handoffs',status:'proposed-for-coordinator-installation',captured_at:new Date().toISOString(),scope:'Canonical source-only read-coverage reconciliation for old captured ICE/ICE-Rust roots. Active-checkout source-map and hash inventories provide no read credit.',method:'Reuse exact attributed ranges from accepted manifests, legacy canonical records, and direct-source supplements. Remove every explicit redaction/unverified exclusion globally before union, regardless of earlier full-read claims. File hashing is metadata verification only. No application code, test, or build was executed.',summary,source_roots:roots,baseline_heads:{ice:baseline.ts.head,rust:baseline.rust.head},baseline_identity_limitations:baselineLimits,coverage_policies:coveragePolicies,identical_copy_proofs:copyProofs,legacy_only_records:Object.entries(files).filter(([,r])=>r.legacy_only_against_accepted_areas).map(([path,r])=>({path,ranges:r.ranges,sha256:r.sha256})),rust_non_crates:Object.entries(files).filter(([p])=>p.startsWith('rust:')&&!p.startsWith('rust:crates/')).map(([path,r])=>({path,ranges:r.ranges,excluded_redacted_lines:r.excluded_redacted_lines,excluded_unverified_lines:r.excluded_unverified_lines})),inputs:[...inputs.values()],files};
const qa={owner:'/root/final_area_handoffs',status:sourceErrors.length||copyIdentityErrors.length||malformed.length||identityConflicts.length?'review-required':'static-reconciliation-computed',summary,source_errors:sourceErrors,copy_identity_errors:copyIdentityErrors,coverage_policies:coveragePolicies,identity_conflicts:identityConflicts,malformed_inputs:malformed,skipped_metadata_or_unscoped:skipped,range_trims:rangeTrims,unmatched_exclusions:unmatchedExclusions,exclusions:exclusionRows,baseline_identity_limitations:baselineLimits,missing_read_hash_paths:Object.entries(files).filter(([,r])=>!r.sha256).map(([p])=>p),legacy_retention_complete:summary.legacy_files===summary.legacy_retained,sessions_627_excluded:files['rust:crates/ice-cli/src/interactive_tui/sessions.rs']?.excluded_unverified_lines.includes(627),limits:['Source-credit inheritance is preserved; this normalizer did not independently review all source bodies.','Current hash metadata does not retroactively certify missing historical dirty-worktree hashes.','No source-map-only records add read coverage.']};
for(const[n,obj]of [['lane-final-areas-read-coverage-proposal.json',output],['lane-final-areas-read-coverage-qa.json',qa]])fs.writeFileSync(path.join(ev,n),JSON.stringify(obj,null,2)+'\n');
console.log(JSON.stringify({summary,source_errors:sourceErrors,identity_conflicts:identityConflicts,malformed_inputs:malformed,unmatched_exclusions:unmatchedExclusions.map(x=>x.path),nonterminal_range_trims:rangeTrims.filter(x=>!x.only_terminal_empty),baseline_limits:baselineLimits.map(x=>x.path),rust_non_crates:output.rust_non_crates}));
