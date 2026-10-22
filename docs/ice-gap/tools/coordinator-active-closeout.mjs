// Root-owned active audit closeout. Writes documentation only; never executes application code.
import fs from'node:fs';import path from'node:path';import crypto from'node:crypto';import{fileURLToPath}from'node:url';import{execFileSync}from'node:child_process';
const B=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..'),E=path.join(B,'evidence'),oldBase='/home/mewtwo/Zks/ice-rust/docs/ice-gap',activeBase='/home/mewtwo/Zykairotis/ice-rust/docs/ice-gap';
const mode=process.argv[2]||'preview',sha=x=>crypto.createHash('sha256').update(x).digest('hex'),read=n=>JSON.parse(fs.readFileSync(path.join(E,n),'utf8'));
if(!['preview','apply','catalog-preview','catalog-apply','render-pending','complete-checkpoint'].includes(mode))throw Error('Unknown mode');
if(![oldBase,activeBase].includes(B))throw Error('Unexpected audit root');
if(mode!=='preview'&&B!==activeBase)throw Error('Active-only operation');
const gate=read('original-checkout-validation.json'),progress=read('progress.json');
if(!gate.passed||gate.phase!=='original'||!gate.source_only||gate.application_or_test_execution)throw Error('Passing original source-only audit required');
if(progress.complete&&mode!=='complete-checkpoint')throw Error('Completed audit is protected');
const areas=['ai','agent','tools','session','settings','extensions','rpc-remote','tui','subagents','safe-verify','observability','cognee','blackhole','providers-auth','cli-packaging','misc'];
const pinned={
 'coordinator-native-prose-replacements.json':'e3c8a805c6cf0c66c669f93285a15a3a107d22eb7365b0cae83c57921eb13476',
 'lane-exports-rpc-ai-auth-native-prose-replacements.json':'3ae59f65990f99fa7d587a6ddfd02ab660c5116c153825fa6342a8d3ab84428e',
 'lane-exports-sub-session-native-prose-replacements.json':'fe7182c6b8c1f7a26ac4b87250dafd104ff5c91d554578b801f334e9b5489423',
 'lane-final-areas-active-identity-proposal.json':'3bad2fa6387b5a5677cd166530547d36f1214467f7fa22b13c1552c6f2846f27',
 'lane-final-areas-first-pass-findings-proposal.json':'435fd3a102c2efa1a08b434a3cae35b9bc55b849b9714e7f49f86e59e8f8b40f',
 'lane-exports-sub-session-cli-prose-review.json':'2da3ccfff955049c4c31620c4199571ee17305081e16cfb0ca83368ab9fca410',
 'lane-exports-sub-session-native-surfaces-review.json':'2321e72183dcb4aee9ec80601a3fec5929200fe7627baef5f1efcf33dc0e8046'
};
const proposals={};for(const[n,h]of Object.entries(pinned)){const raw=fs.readFileSync(path.join(E,n));if(sha(raw)!==h)throw Error('Reviewed proposal hash changed '+n);proposals[n]=JSON.parse(raw);}
let projection=null;
async function prepareProjection(){
 const originalPreview=B===oldBase;
 const reportName=originalPreview?'relocation-rust-remap-preview.json':'relocation-rust-remap.json';
 const reportRaw=fs.readFileSync(path.join(E,reportName)),report=JSON.parse(reportRaw);
 const mapRaw=fs.readFileSync(path.join(E,'active-checkout-source-map.json')),map=JSON.parse(mapRaw);
 const hunkRaw=fs.readFileSync(path.join(E,'active-checkout-diff-hunks-complete.json')),hunks=JSON.parse(hunkRaw);
 const toolHash=sha(fs.readFileSync(path.join(B,'tools/remap-rust-citations.mjs'))),reviewRaw=fs.readFileSync(path.join(E,'lane-final-areas-remap-review.json')),review=JSON.parse(reviewRaw);
 if(toolHash!=='aef378c0e58ee2999e81318f7287f3bf9dc4eca3f589f540c75e8e033d40e5be'||report.remapper_sha256!==toolHash||sha(mapRaw)!==report.source_map_sha256||sha(hunkRaw)!==report.hunk_map_sha256)throw Error('Reviewed source mapping changed');
 if(sha(reviewRaw)!=='ebf9835bd323170cb3c716b91a6f2101ce8b8c66a0e9eb204bf98207cd3f59f5'||review.status!=='passed'||review.errors.length)throw Error('Independent remapper review changed');
 if(report.old_root!==path.resolve(oldBase,'../..')||report.active_root!==path.resolve(activeBase,'../..')||report.old_head!=='ff9270f26e263ea9ce54349786bf2d93c4517476'||report.active_head!=='1891337558105fb8322c79617e5a03d98c1b2c3a')throw Error('Unexpected remap identity');
 for(const input of report.review_inputs)if(sha(fs.readFileSync(path.join(B,input.path)))!==input.sha256)throw Error('Remap review input changed '+input.path);
 if(!originalPreview){const receipt=read('active-relocation-verification.json');if(!receipt.all_rust_citations_mapped_and_checked||receipt.active_head!==report.active_head)throw Error('Apply native prose only after the completed citation remap');}
 const git=(root,...args)=>execFileSync('git',['--no-optional-locks','-C',root,...args],{encoding:'utf8',stdio:['ignore','pipe','pipe']}).trim();
 for(const side of['old','active']){const identity=map.identities[side];if(git(identity.root,'rev-parse','HEAD')!==identity.head||git(identity.root,'branch','--show-current')!==identity.branch||git(identity.root,'diff','--name-only','HEAD','--','crates','scripts','Cargo.toml','Cargo.lock'))throw Error('Source identity changed before proposal projection');}
 const byOld=new Map(map.rows.map(r=>[r.old_path,r])),byActive=new Map(map.rows.map(r=>[r.active_path,r])),changes=new Map(hunks.rows.map(r=>[r.old_path,r.changes??[]]));
 const certified=new Map(report.source_identities.map(r=>[r.old_path,r]));
 function range(rel,a,b,alreadyActive=false){
  const m=(alreadyActive?byActive:byOld).get(rel);if(!m?.active?.present)throw Error('Missing proposal source mapping '+rel);
  if(alreadyActive){if(!(1<=a&&a<=b&&b<=m.active.total_lines))throw Error('Already-active proposal bounds');return{old_path:rel,active_path:rel,old_range:[a,b],new_range:[a,b],method:'unchanged'};}
  const s=certified.get(rel),cs=['identical','renamed_identical'].includes(m.status)?[]:(changes.get(rel)??[]);
  if(!m.old?.present||!s||s.old_sha256!==m.old.sha256||s.active_sha256!==m.active.sha256||s.changes!==cs.length)throw Error('Proposal file lacks complete saved source-map certification '+rel);
  if(!(1<=a&&a<=b&&b<=m.old.total_lines))throw Error('Original proposal bounds '+rel+':'+a+'-'+b);
  function point(n,end){let offset=0;for(const c of cs){if(c.old_count===0){if(n>c.old_start)offset+=c.active_count;else break;continue;}if(n<c.old_start)break;if(n>=c.old_start+c.old_count){offset+=c.active_count-c.old_count;continue;}if(c.active_count===0)throw Error('Proposal citation endpoint deleted '+rel+':'+n);return end?c.active_start+c.active_count-1:c.active_start;}return n+offset;}
  const na=point(a,false),nb=point(b,true),overlap=cs.some(c=>c.old_count===0?a<=c.old_start&&b>c.old_start:a<=c.old_start+c.old_count-1&&b>=c.old_start);
  if(!(1<=na&&na<=nb&&nb<=m.active.total_lines))throw Error('Active proposal bounds '+rel+':'+na+'-'+nb);
  return{old_path:rel,active_path:m.active_path,old_range:[a,b],new_range:[na,nb],method:overlap||rel!==m.active_path?'reviewed':na!==a||nb!==b?'shifted':'unchanged'};
 }
 let checked=0;for(const c of report.citations){const r=range(c.old_path,...c.old_range,c.original_coordinate_space==='active');for(const k of['active_path','old_range','new_range','method'])if(JSON.stringify(r[k])!==JSON.stringify(c[k]))throw Error('Projection disagrees with reviewed citation '+c.old_citation+' at '+k);checked++;}
 const spacingTool=path.join(B,'tools/fix-spacing.mjs'),spacingHash=sha(fs.readFileSync(spacingTool));
 const spacingReportName='spacing-residual-validation.json',spacingReportPath=path.join(E,spacingReportName);
 if(!fs.existsSync(spacingReportPath))throw Error('Apply and validate reviewed residual spacing before native projection');
 const spacingRaw=fs.readFileSync(spacingReportPath),spacingReport=JSON.parse(spacingRaw);
 if(spacingReport.applied!==true||spacingReport.passed!==true||spacingReport.citations_byte_identical!==true||spacingReport.identifiers_byte_identical!==true||spacingReport.protected_spans_byte_identical!==true||spacingReport.remaining_real_prose_glue!==0||spacingReport.tool_sha256!==spacingHash)throw Error('Positive residual spacing proof for this exact import-safe helper is required');
 const spacing=await import(spacingTool);
 if(typeof spacing.normalizeResidualProse!=='function'||sha(fs.readFileSync(spacingTool))!==spacingHash)throw Error('Pure reviewed residual normalizer required');
 const citations=[];
 function mapText(text,context={}){
  return text.replace(/\b(rust|rust-active):([A-Za-z0-9_./@+\-]+):(\d+)(?:[-–](\d+))?/g,(old,side,rel,a,b)=>{
   const r=range(rel,+a,+(b??a),side==='rust-active'),next='rust:'+r.active_path+':'+r.new_range[0]+(b!==undefined||r.new_range[1]!==r.new_range[0]?'-'+r.new_range[1]:'');
   citations.push({context,old_citation:old,new_citation:next,...r});return next;
  });
 }
 function transform(value,context){if(typeof value!=='string')return value;const normalized=spacing.normalizeResidualProse(value,context);if(typeof normalized!=='string'||spacing.normalizeResidualProse(normalized,context)!==normalized)throw Error('Residual normalizer must return an idempotent string');return mapText(normalized,context);}
 const preserved=new Set(['reference_normalization','read_sources','source_review','read_evidence','reads','read_coverage','hashes','inputs','provenance','provenance_targets','provenance_review','provenance_evidence','frozen_checkout_finding','baseline_identity_limitations','source_label_repairs']);
 function walk(x,context){if(typeof x==='string')return mapText(x,context);if(Array.isArray(x))return x.map(v=>walk(v,context));if(x&&typeof x==='object')return Object.fromEntries(Object.entries(x).map(([k,v])=>[k,preserved.has(k)?v:walk(v,context)]));return x;}
 function virtualDocument(rel,value){
  if(rel.startsWith('evidence/spec-'))return walk(value,{artifact:rel,scope:'virtual-remapped-current-document'});
  if(rel==='evidence/qa-first-pass-coverage.json')return{...value,claims:value.claims.map(c=>({...c,evidence:walk(c.evidence??[],{artifact:rel,id:c.id}),finding:mapText(c.finding??'',{artifact:rel,id:c.id})}))};
  const out={...value};for(const k of['entries','claims','rust_only','open_questions','unverified','surfaces','candidate_exclusions'])if(k in out)out[k]=walk(out[k],{artifact:rel,scope:k});return out;
 }
 return{originalPreview,transform,virtualDocument,citations,bound:{coordinate_space:'active',virtual_original_preview:originalPreview,report:reportName,report_sha256:sha(reportRaw),remapper_sha256:toolHash,source_map_sha256:sha(mapRaw),hunk_map_sha256:sha(hunkRaw),independent_review_sha256:sha(reviewRaw),reviewed_range_records_checked:checked,spacing_tool_sha256:spacingHash,spacing_validation:spacingReportName,spacing_validation_sha256:sha(spacingRaw),old_head:report.old_head,active_head:report.active_head}};
}

const documents=new Map(),changes=[],touchedAreas=new Set();
function document(rel){
 if(!/^evidence\/[A-Za-z0-9_-]+\.json$/.test(rel))throw Error('Unexpected document '+rel);
 if(!documents.has(rel)){const before=fs.readFileSync(path.join(B,rel),'utf8'),value=JSON.parse(before);documents.set(rel,{before,value:projection?.originalPreview?projection.virtualDocument(rel,value):value});}return documents.get(rel).value;
}
function replace(rel,pointer,oldValue,newValue,proposal,reason){
 const parts=pointer.split('/').slice(1).map(p=>p.replaceAll('~1','/').replaceAll('~0','~'));
 let obj=document(rel);for(const key of parts.slice(0,-1)){if(obj?.[key]===undefined)throw Error('Missing pointer '+rel+pointer);obj=obj[key];}
 const key=parts.at(-1);
 if(projection&&proposal!=='coordinator accepted spec'){const context={artifact:rel,entry_id:obj?.id??obj?.entry_id??null,id:obj?.id??obj?.entry_id??null,field:key,pointer};oldValue=projection.transform(oldValue,context);newValue=projection.transform(newValue,context);}
 if(JSON.stringify(obj[key])!==JSON.stringify(oldValue))throw Error('Stale exact field '+rel+pointer);
 if(typeof oldValue!==typeof newValue)throw Error('Replacement type changed '+rel+pointer);
 changes.push({artifact:rel,pointer,old_value:oldValue,new_value:newValue,proposal,reason});obj[key]=newValue;
}
function specChange(area,id,field,oldValue,newValue,proposal,reason){
 if(!areas.includes(area))throw Error('Unknown area');const rel='evidence/spec-'+area+'.json',s=document(rel),index=s.entries.findIndex(e=>e.id===id);if(index<0)throw Error('Unknown entry '+id);
 replace(rel,'/entries/'+index+'/'+field,oldValue,newValue,proposal,reason);touchedAreas.add(area);
}
function normalizedRel(rel){return rel.startsWith('docs/ice-gap/')?rel.slice(13):rel;}
function commitPlan(kind){
 const files=[...documents].map(([rel,{before,value}])=>({path:rel,before_sha256:sha(before),after_sha256:sha(JSON.stringify(value,null,2)+'\n')}));
 const stable={tool_sha256:sha(fs.readFileSync(fileURLToPath(import.meta.url))),proposals:pinned,projection:projection?.bound??null,files,changes};
 const report={schema_version:1,captured_at:new Date().toISOString(),mode,source_only:true,application_or_test_execution:false,original_gate_sha256:sha(fs.readFileSync(path.join(E,'original-checkout-validation.json'))),...stable};
 if(mode.endsWith('apply')){
  const preview=read(kind+'-preview.json');for(const k of['tool_sha256','proposals','projection','files','changes','original_gate_sha256'])if(JSON.stringify(preview[k])!==JSON.stringify(report[k]))throw Error('Fresh reviewed preview differs '+k);
  for(const f of files)if(sha(fs.readFileSync(path.join(B,f.path)))!==f.before_sha256)throw Error('Concurrent change '+f.path);
  for(const[rel,{value}]of documents)fs.writeFileSync(path.join(B,rel),JSON.stringify(value,null,2)+'\n');
  if(kind==='active-native-prose')fs.writeFileSync(path.join(E,'active-native-render-pending.json'),JSON.stringify({source_only:true,complete:false,areas:[...touchedAreas],spec_files:files.filter(f=>f.path.startsWith('evidence/spec-')),reason:'Native changes are applied after citation remap and before helper migration. Run this helper in render-pending mode after the renderer is migrated.'},null,2)+'\n');
  fs.writeFileSync(path.join(E,kind+'-accepted.json'),JSON.stringify(report,null,2)+'\n');
 }else fs.writeFileSync(path.join(E,kind+'-preview.json'),JSON.stringify(report,null,2)+'\n');
 if(projection&&!mode.endsWith('apply'))fs.writeFileSync(path.join(E,'lane-native-final-projection-qa.json'),JSON.stringify({captured_at:new Date().toISOString(),passed:true,source_only:true,application_or_test_execution:false,tool_sha256:stable.tool_sha256,proposal_hashes:pinned,projection:projection.bound,exact_field_guards:changes.length,projection_citation_occurrences:projection.citations.length,citations:projection.citations,rendering_deferred_until_migration:true,immutable_proposals_unchanged:true},null,2)+'\n');
 console.log(JSON.stringify({mode,changes:changes.length,files:files.length,areas:[...touchedAreas],output:'evidence/'+kind+(mode.endsWith('apply')?'-accepted.json':'-preview.json')},null,2));
}
if(['preview','apply'].includes(mode)){
 projection=await prepareProjection();
 for(const r of proposals['coordinator-native-prose-replacements.json'].changes)specChange('cli-packaging',r.id,r.field,r.old_text,r.new_text,'coordinator-native-prose-replacements.json',r.reason);
 for(const r of proposals['lane-exports-rpc-ai-auth-native-prose-replacements.json'].replacements)specChange(r.area,r.entry_id,r.field,r.old_text,r.new_text,'lane-exports-rpc-ai-auth-native-prose-replacements.json',r.reason);
 const sub=proposals['lane-exports-sub-session-native-prose-replacements.json'];
 for(const r of [...sub.field_replacements,...sub.manifest_replacements]){
  if(r.expected_old_field.split(r.old_text).length-1!==r.expected_occurrences||r.expected_old_field.replaceAll(r.old_text,r.new_text)!==r.proposed_new_field)throw Error('Substring proposal mismatch');
  if(r.field)specChange(r.area,r.id,r.field,r.expected_old_field,r.proposed_new_field,'lane-exports-sub-session-native-prose-replacements.json',r.reason);
  else replace(normalizedRel(r.path),r.pointer,r.expected_old_field,r.proposed_new_field,'lane-exports-sub-session-native-prose-replacements.json',r.reason);
 }
 const final=proposals['lane-final-areas-active-identity-proposal.json'];
 if(final.entry_updates.length)throw Error('Unexpected unreviewed metadata updates');
 for(const r of final.field_replacements)specChange(r.area,r.id,r.field,r.old_text,r.new_text,'lane-final-areas-active-identity-proposal.json',r.reason);
 for(const r of final.manifest_replacements)replace(normalizedRel(r.artifact),r.json_pointer,r.old_text,r.new_text,'lane-final-areas-active-identity-proposal.json',r.reason);
 for(const r of proposals['lane-final-areas-first-pass-findings-proposal.json'].replacements)replace(normalizedRel(r.artifact),r.json_pointer,r.old_text,r.new_text,'lane-final-areas-first-pass-findings-proposal.json',r.reason);
 for(const area of touchedAreas){
  const spec=document('evidence/spec-'+area+'.json'),rel='evidence/'+area+'.json',manifest=document(rel);
  for(const e of spec.entries){const i=manifest.entries.findIndex(x=>x.id===e.id);if(i<0)throw Error('Missing accepted metadata '+e.id);
   for(const k of['title','status','priority','size_min','size_max','dependencies'])if(JSON.stringify(manifest.entries[i][k])!==JSON.stringify(e[k]))replace(rel,'/entries/'+i+'/'+k,manifest.entries[i][k],e[k],'coordinator accepted spec','Synchronize accepted index metadata with the reviewed active specification.');
  }
 }
 commitPlan('active-native-prose');
}else if(mode.startsWith('catalog-')){
 const relocation=read('active-relocation-verification.json');if(!relocation.all_rust_citations_mapped_and_checked||relocation.active_head!=='1891337558105fb8322c79617e5a03d98c1b2c3a')throw Error('Complete citation remap required first');
 const sub=proposals['lane-exports-sub-session-native-prose-replacements.json'],value=sub.proposed_catalog_object;
 if(value.id!=='RUST-069'||value.entry_ids.length||!sub.catalog_insertion.apply_after_existing_citation_remap)throw Error('Unexpected catalog proposal');
 const map=read('active-checkout-source-map.json'),root=path.resolve(B,'../..');
 const fixedSources=[['crates/ice-cli/src/prebuilt_release.rs',11,'https://github.com/zyk-void/ice-rust'],['crates/ice-cli/src/changelog.rs',27,'https://raw.githubusercontent.com/zyk-void/ice-rust/main/CHANGELOG.md']],proof=[];
 for(const[rel,line,expectedLiteral]of fixedSources){
  const raw=fs.readFileSync(path.join(root,rel)),m=map.rows.find(r=>r.active_path===rel);
  if(sha(raw)!==m?.active?.sha256||!raw.toString('utf8').split('\n')[line-1].includes(expectedLiteral))throw Error('Fixed repository source changed '+rel);
  proof.push({path:rel,line,sha256:sha(raw),text:raw.toString('utf8').split('\n')[line-1]});
 }
 const valueRaw=fs.readFileSync(path.join(root,sub.proposed_provenance_addendum.file));if(sha(valueRaw)!==sub.proposed_provenance_addendum.current_sha256)throw Error('Session value source changed');
 const rel='evidence/rust-defects.json',catalog=document(rel);if(catalog.some(d=>d.id==='RUST-069'))throw Error('Catalog already inserted');
 const i=catalog.findIndex(d=>d.id==='RUST-045');if(i<0)throw Error('Missing RUST-045');
 const oldRaw=fs.readFileSync(path.join(oldBase,rel));if(sha(oldRaw)!==sub.catalog_insertion.current_catalog_snapshot_sha256)throw Error('Original catalog changed');
 const frozen=JSON.parse(oldRaw).find(d=>d.id==='RUST-045'),prior=catalog[i];
 const fixed={...prior,title:"Historical mixed-case official repository rejection, fixed in the active checkout",frozen_checkout_finding:frozen,origin_provenance_status:frozen.provenance_status,provenance_status:'fixed-in-active-checkout',active_status:'fixed',active_head:relocation.active_head,
  sources:[...new Set([...prior.sources,...fixedSources.map(([p,l])=>'rust-active:'+p+':'+l)])],
  behavior:'The active native source uses a canonical lowercase official repository and no longer has the mixed-case rejection recorded in the frozen checkout. The original source-level finding and its local Git attribution remain preserved.',
  technical_explanation:'REPOSITORY is https://github.com/zyk-void/ice-rust at prebuilt_release.rs:11, matching canonical_repository for manifest validation and the repository passed to release-asset URL validation. The changelog constant at changelog.rs:27 uses the same lowercase owner. This resolves the specific repository-case mismatch; remote asset existence and successful download/activation were not tested.',
  assumptions:'The correction is verified from the fixed active source and saved changed-hunk review. No network, application, download, platform activation or test execution was performed. The frozen finding remains historical evidence.',
  provenance:prior.provenance.replace('Active-checkout correction remains a final relocation check.','The active-checkout constants were confirmed corrected during final relocation; the original provenance record is preserved below.'),
  active_resolution:{status:'fixed',source_only:true,head:relocation.active_head,sources:proof,review:'evidence/coordinator-native-hunk-review.json'}
 };
 replace(rel,'/'+i,prior,fixed,'coordinator native source review','Preserve the historical defect and identify its fixed active state.');
 catalog.push({...structuredClone(value),active_status:'open-conditional',active_head:relocation.active_head});changes.push({artifact:rel,pointer:'/'+(catalog.length-1),operation:'append',id:'RUST-069',proposal:'lane-exports-sub-session-native-prose-replacements.json',reason:'Reviewed conditional native compatibility finding with already-active citations.'});
 const provRel='evidence/rust-defect-provenance.json',prov=document(provRel),oldProvRaw=fs.readFileSync(path.join(oldBase,provRel));
 if(sha(oldProvRaw)!==sub.catalog_insertion.current_provenance_snapshot_sha256)throw Error('Original provenance changed');
 const frozenProv=JSON.parse(oldProvRaw);for(const[k,v]of Object.entries(frozenProv))if(JSON.stringify(prov[k])!==JSON.stringify(v))throw Error('Frozen provenance field changed '+k);
 if(prov.active_addenda?.some(r=>r.id==='RUST-069'))throw Error('Duplicate provenance');
 prov.active_addenda=[...(prov.active_addenda??[]),structuredClone(sub.proposed_provenance_addendum)];
 prov.active_addenda_note='Original root, head, targets, excerpts and attribution retain frozen-checkout coordinates. Each active_addenda item identifies its own active source and local Git scope.';
 changes.push({artifact:provRel,operation:'append-active-addendum',id:'RUST-069',preserved_original_targets:true});
 commitPlan('active-native-defects');
}else if(mode==='render-pending'){
 const pending=read('active-native-render-pending.json');if(pending.complete)throw Error('Pending native render already completed');
 const renderer=path.join(B,'tools/render-area.mjs'),migration=read('lane-exports-rpc-ai-auth-migration-result.json');if(!migration.applied||!fs.existsSync(renderer))throw Error('Completed helper migration required before rendering');
 for(const f of pending.spec_files)if(sha(fs.readFileSync(path.join(B,f.path)))!==f.after_sha256)throw Error('Spec changed before pending render '+f.path);
 for(const area of pending.areas)execFileSync(process.execPath,[renderer,area],{cwd:path.resolve(B,'../..'),stdio:'pipe'});
 pending.complete=true;pending.rendered_at=new Date().toISOString();pending.renderer_sha256=sha(fs.readFileSync(renderer));pending.outputs=pending.areas.map(a=>({path:a+'.md',sha256:sha(fs.readFileSync(path.join(B,a+'.md')))}));fs.writeFileSync(path.join(E,'active-native-render-pending.json'),JSON.stringify(pending,null,2)+'\n');console.log(JSON.stringify({mode,complete:true,areas:pending.areas}));
}else{
 const g=read('final-validation.json');if(!progress.complete||!g.passed||g.phase!=='active')throw Error('Complete active audit must already pass');
 if(read('active-native-render-pending.json').complete!==true)throw Error('Complete native rendering required before final checkpoint');
 const incidentPath=path.join(E,'lane-native-final-spacing-preview-incident.json'),incident=fs.existsSync(incidentPath)?JSON.parse(fs.readFileSync(incidentPath,'utf8')):null;
 for(const a of areas)for(const n of['spec-'+a+'.json',a+'.json']){const d=read(n);if(!d.complete||!d.draft_complete||d.accepted_phase!=='active-source-verified')throw Error('Incomplete metadata '+n);}
 const stamp=new Date().toISOString(),text=[
  '# Resume checkpoint — completed active audit '+stamp, '', '## Current coordinator execution (complete)', '',
  '| Lane | State | Result |','|---|---|---|',
  '| L1 / L2 publication | published | All 16 areas; 791 entries |',
  '| L3 / L4 exports | merged | 1,654 reviewed records; zero unresolved |',
  '| L5 surfaces | merged | 3,975 rows; zero pending |',
  '| L6 dependencies | merged | 3,053 typed edges; all IDs resolve |',
  '| L7 claims and index | verified | 160 claims; complete README, graph, order and limitations |',
  '| L8 readability | verified | Original spacing validation/review and residual proof preserved |',
  '| Active citations and read credit | verified | Fixed-head remap with reviewed changes and preserved exclusions |',
  '| Native corrections | merged | RUST-045 fixed; RUST-069 conditional active compatibility finding |',
  '| Helper migration | verified | Helpers under tools; safe syntax and documentation validators passed |',
  '| Final machine gate | verified | '+g.captured_at+'; zero errors; '+g.warnings.length+' terminal-empty-line warnings |','',
  'Reserved IDs: none pending. RUST-069 is published. SESSION-039 and SESSION-040 remain intentionally unused.','',
  'Pending merges: none. No audit closeout work remains. Product-intent decisions and runtime/external verification limits are explicitly listed in README.md and evidence/open-questions.json.','',
  'Original audit evidence is retained at '+oldBase+'. Active deliverable is '+activeBase+'. No application source, commits, installs, network operations or private runtime directories were touched.','',
  ...(incident?.original_dry_run_unavailable&&!incident?.original_dry_run_restored?['Evidence integrity note: the original spacing dry-run file was overwritten by a premature legacy-helper import and could not be recovered byte-for-byte. The original full validation (5,359 changes) and its review remain intact; evidence/lane-native-final-spacing-preview-incident.json records the expected and replacement hashes. The replacement dry-run is not historical proof.','']:[]),
  '---',''
 ].join('\n');
 const resumePath=path.join(E,'RESUME.md'),resume=fs.readFileSync(resumePath,'utf8'),divider=/\r?\n---\r?\n/.exec(resume);
 if(!divider)throw Error('Current checkpoint header separator missing');
 const historicalCheckpoints=resume.slice(divider.index+divider[0].length);
 fs.writeFileSync(resumePath,text+historicalCheckpoints);
 fs.writeFileSync(path.join(E,'closeout-completion.json'),JSON.stringify({captured_at:stamp,complete:true,final_gate:'evidence/final-validation.json',final_gate_sha256:sha(fs.readFileSync(path.join(E,'final-validation.json'))),final_gate_captured_at:g.captured_at,entries:g.counts.entry_count,areas:16,source_only:true,application_or_test_execution:false,pending_merges:[],resume_header_replaced:true,historical_checkpoints_retained:true,historical_checkpoints_sha256:sha(historicalCheckpoints),evidence_integrity_incident:incident?'evidence/lane-native-final-spacing-preview-incident.json':null,limits:g.limits},null,2)+'\n');
 console.log(JSON.stringify({complete:true,stamp,gate:g.captured_at,errors:g.issues.length,warnings:g.warnings.length}));
}
