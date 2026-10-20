// Exact fixed-head citation and read-credit relocation. No application/test execution.
import fs from'node:fs';import path from'node:path';import crypto from'node:crypto';import{fileURLToPath}from'node:url';import{execFileSync}from'node:child_process';
const B=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..'),E=path.join(B,'evidence'),mode=process.argv[2]||'preview';
if(!['preview','apply'].includes(mode))throw Error('preview|apply');
const read=n=>JSON.parse(fs.readFileSync(path.join(E,n),'utf8')),sha=x=>crypto.createHash('sha256').update(x).digest('hex');
const map=read('active-checkout-source-map.json'),diff=read('active-checkout-diff-hunks-complete.json'),oldRoot=map.identities.old.root,activeRoot=map.identities.active.root;
const gate=read('original-checkout-validation.json');
if(!gate.passed||gate.phase!=='original'||!gate.source_only||gate.application_or_test_execution)throw Error('Passing original source-only gate required');
if(read('progress.json').complete)throw Error('Published complete tree is protected');
if(mode==='apply'&&B!==path.join(activeRoot,'docs/ice-gap'))throw Error('Apply only in active audit copy');
const git=(r,...a)=>execFileSync('git',['--no-optional-locks','-C',r,...a],{encoding:'utf8',stdio:['ignore','pipe','pipe']}).trim();
for(const [root,identity]of[[oldRoot,map.identities.old],[activeRoot,map.identities.active]]){
 if(git(root,'rev-parse','HEAD')!==identity.head||git(root,'branch','--show-current')!==identity.branch)throw Error('Checkout identity drift');
 if(git(root,'diff','--name-only','HEAD','--','crates','scripts','Cargo.toml','Cargo.lock'))throw Error('Native source worktree changed');
}
const areas=['ai','agent','tools','session','settings','extensions','rpc-remote','tui','subagents','safe-verify','observability','cognee','blackhole','providers-auth','cli-packaging','misc'];
const metadata=new Map(map.rows.map(r=>[r.old_path,r])),activeMetadata=new Map(map.rows.map(r=>[r.active_path,r])),changesByPath=new Map(diff.rows.map(r=>[r.old_path,r]));
const approved=new Map(),reviewInputs=[];
function key(c){return[c.old_start,c.old_count,c.active_start,c.active_count].join(':');}
function addReview(file,c,proof,oldHash,activeHash,confirmed){
 if(!confirmed)throw Error('Unconfirmed review '+file);
 const m=metadata.get(file);if(!m||m.old.sha256!==oldHash||m.active.sha256!==activeHash)throw Error('Review identity mismatch '+file);
 const list=approved.get(file)??new Map();if(list.has(key(c))&&list.get(key(c)).old_sha256!==oldHash)throw Error('Review conflict');
 list.set(key(c),{artifact:proof,old_sha256:oldHash,active_sha256:activeHash});approved.set(file,list);
}
for(const name of['coordinator-native-hunk-review.json','lane-exports-rpc-ai-auth-native-hunk-review.json','lane-exports-sub-session-native-hunk-review.json','lane-final-areas-active-noncrates-review.json']){
 const raw=fs.readFileSync(path.join(E,name)),d=JSON.parse(raw);reviewInputs.push({path:'evidence/'+name,sha256:sha(raw)});
 if(name.startsWith('coordinator-'))for(const f of d.files)for(const h of f.hunks)addReview(f.old_path,h,name,f.old_sha256,f.active_sha256,h.confirmed);
 else if(name.includes('rpc-ai-auth'))for(const f of d.files)for(const h of f.hunks)for(const c of h.changed_ranges)addReview(f.old_path,c,name,f.old_sha256,f.active_sha256,['confirmed','confirmed-with-documentation-caveat'].includes(h.review_status)&&!(h.redactions??[]).length);
 else if(name.includes('sub-session'))for(const f of d.rows)for(const c of f.changes)addReview(f.old_path,c,name,f.old.sha256,f.active.sha256,c.reviewed);
 else for(const c of d.reviews)addReview(c.old_path,c,name,c.old_sha256,c.active_sha256,!!c.confirmation);
}
const renameName='lane-exports-rpc-ai-auth-bin-rename-confirmation.json',renameRaw=fs.readFileSync(path.join(E,renameName));
if(sha(renameRaw)!=='0f1f331067221df7b7a58503a079001f5eb7797e33336d3b65ace7ca7c9df96b')throw Error('Rename confirmation changed');
reviewInputs.push({path:'evidence/'+renameName,sha256:sha(renameRaw)});
const sourceCache=new Map(),validatedFiles=new Map();
function fileData(root,rel){
 if(path.isAbsolute(rel)||rel.split('/').includes('..')||/^(?:\.ice|\.icers|\.git|\.aws|\.ssh)\//.test(rel))throw Error('Unsafe source path');
 const p=path.join(root,rel);if(sourceCache.has(p))return sourceCache.get(p);
 const raw=fs.readFileSync(p),text=raw.toString('utf8'),split=text.split('\n'),physical=split.at(-1)===''?split.slice(0,-1):split;
 const d={sha256:sha(raw),split_lines:split.length,lines:physical.length,text_lines:physical};sourceCache.set(p,d);return d;
}
function info(rel){
 if(validatedFiles.has(rel))return validatedFiles.get(rel);
 const m=metadata.get(rel);if(!m||!m.old?.present||!m.active?.present)throw Error('Missing/deleted file mapping '+rel);
 const old=fileData(oldRoot,rel),active=fileData(activeRoot,m.active_path);
 if(old.sha256!==m.old.sha256||active.sha256!==m.active.sha256)throw Error('Current source hash differs from fixed map '+rel);
 let cs=changesByPath.get(rel)?.changes??[];
 if(m.status==='identical'||m.status==='renamed_identical'){
  if(old.sha256!==active.sha256)throw Error('False identity mapping '+rel);
  cs=[];
 }else{
  if(!changesByPath.has(rel))throw Error('Missing captured hunks '+rel);
  for(const c of cs)if(!approved.get(rel)?.has(key(c)))throw Error('Unreviewed changed segment '+rel+':'+key(c));
 }
 if(m.active_path!==rel){
  if(rel!=='crates/ice-cli/src/bin/ice.rs'||m.active_path!=='crates/ice-cli/src/bin/icers.rs'||old.sha256!=='0edb0f556b231d6a9c2c77df539c029851ab48a05be2084e4ee6cd797dd3fdbf')throw Error('Unreviewed rename '+rel);
 }
 const d={old_path:rel,active_path:m.active_path,status:m.status,old,active,changes:cs};validatedFiles.set(rel,d);return d;
}
function point(n,d,end=false){
 let offset=0;
 for(const c of d.changes){
  if(c.old_count===0){if(n>c.old_start)offset+=c.active_count;else break;continue;}
  if(n<c.old_start)break;
  if(n>=c.old_start+c.old_count){offset+=c.active_count-c.old_count;continue;}
  if(c.active_count===0)throw Error('Citation endpoint deleted; individual review required '+d.old_path+':'+n);
  return end?c.active_start+c.active_count-1:c.active_start;
 }
 return n+offset;
}
function overlap(c,a,b){return c.old_count===0?a<=c.old_start&&b>c.old_start:a<=c.old_start+c.old_count-1&&b>=c.old_start;}
function mappedRange(rel,a,b){
 const d=info(rel);if(!(1<=a&&a<=b&&b<=d.old.split_lines))throw Error('Old citation bounds '+rel+':'+a+'-'+b);
 const overlapping=d.changes.filter(c=>overlap(c,a,b)),na=point(a,d,false),nb=point(b,d,true);
 if(!(1<=na&&na<=nb&&nb<=d.active.split_lines))throw Error('Active citation bounds '+rel+':'+na+'-'+nb);
 return{old_path:rel,active_path:d.active_path,old_range:[a,b],new_range:[na,nb],method:overlapping.length||d.active_path!==rel?'reviewed':na!==a||nb!==b?'shifted':'unchanged',overlapping_changes:overlapping.map(c=>({...c,review:approved.get(rel).get(key(c))})),rename_review:d.active_path!==rel?renameName:undefined};
}
const citations=[],planned=[],inputHashes=[],alreadyActiveFiles=new Map();
function activeRange(rel,a,b){
 const m=activeMetadata.get(rel);if(!m?.active?.present)throw Error('Missing already-active citation file '+rel);
 const d=fileData(activeRoot,rel);if(d.sha256!==m.active.sha256)throw Error('Already-active citation source hash drift '+rel);
 if(!(1<=a&&a<=b&&b<=d.split_lines))throw Error('Already-active citation bounds '+rel+':'+a+'-'+b);
 alreadyActiveFiles.set(rel,{active_path:rel,active_sha256:d.sha256});
 return{old_path:rel,active_path:rel,old_range:[a,b],new_range:[a,b],method:'unchanged',original_coordinate_space:'active',verified_active_sha256:d.sha256,overlapping_changes:[]};
}
function mapText(text,document,location){
 return text.replace(/\b(rust|rust-active):([A-Za-z0-9_./@+\-]+):(\d+)(?:[-–](\d+))?/g,(old,side,p,a,b)=>{
  const r=side==='rust-active'?activeRange(p,+a,+(b??a)):{...mappedRange(p,+a,+(b??a)),original_coordinate_space:'original'},range=r.new_range,next='rust:'+r.active_path+':'+range[0]+(b!==undefined||range[1]!==range[0]?'-'+range[1]:'');
  citations.push({document,location,old_citation:old,new_citation:next,...r});return next;
 });
}
function walk(x,document,loc='',skip=new Set()){
 if(typeof x==='string')return mapText(x,document,loc);
 if(Array.isArray(x))return x.map((v,i)=>walk(v,document,loc+'/'+i,skip));
 if(x&&typeof x==='object')return Object.fromEntries(Object.entries(x).map(([k,v])=>[k,skip.has(k)?v:walk(v,document,loc+'/'+k,skip)]));
 return x;
}
function queue(rel,object,before){const after=typeof object==='string'?object:JSON.stringify(object,null,2)+'\n';planned.push({path:rel,before_sha256:sha(before),after_sha256:sha(after),changed:before!==after,after});inputHashes.push({path:rel,sha256:sha(before)});}
const preservedKeys=new Set(['reference_normalization','read_sources','source_review','read_evidence','reads','read_coverage','hashes','inputs','provenance','provenance_targets','provenance_review','provenance_evidence','frozen_checkout_finding','baseline_identity_limitations','source_label_repairs']);
for(const a of areas){
 for(const name of['spec-'+a+'.json',a+'.json']){
  const rel='evidence/'+name,before=fs.readFileSync(path.join(B,rel),'utf8'),v=JSON.parse(before);
  let next;
  if(name.startsWith('spec-'))next=walk(v,rel,'',preservedKeys);
  else{
   next={...v};
   for(const k of['entries','claims','rust_only','open_questions','unverified','surfaces','candidate_exclusions'])if(k in next)next[k]=walk(next[k],rel,'/'+k,preservedKeys);
   for(let i=0;i<(next.surfaces??[]).length;i++){
    const s=next.surfaces[i];if(s.rust_ref){const r=mappedRange(s.rust_ref.slice(0,s.rust_ref.lastIndexOf(':')),+s.rust_ref.split(':').at(-1),+s.rust_ref.split(':').at(-1));const old=s.rust_ref;s.rust_ref=r.active_path+':'+r.new_range[0];citations.push({document:rel,location:'/surfaces/'+i+'/rust_ref',old_citation:'rust:'+old,new_citation:'rust:'+s.rust_ref,...r});}
   }
   next.source_evidence_coordinate_system='Historical read/provenance records retain ff9270f coordinates and hashes; current rust: comparison citations and surfaces are remapped to 1891337. Canonical read-coverage.json separately records current source identity and exact transferred credit.';
  }
  queue(rel,next,before);
 }
}
for(const name of['rust-defects.json','qa-first-pass-coverage.json']){
 const rel='evidence/'+name,before=fs.readFileSync(path.join(B,rel),'utf8'),v=JSON.parse(before);let next;
 if(name==='rust-defects.json')next=v.map(d=>walk(d,rel,'/'+d.id,preservedKeys));
 else{next={...v,claims:v.claims.map(c=>({...c,evidence:walk(c.evidence??[],rel,'/'+c.id+'/evidence'),finding:mapText(c.finding??'',rel,'/'+c.id+'/finding')}))};next.active_comparison={root:activeRoot,head:map.identities.active.head,method:'Current evidence/finding citations remapped. Report excerpts, versioned report_refs, input hashes and historical read attribution are unchanged.'};}
 queue(rel,next,before);
}
for(const a of[...areas,'README']){
 const rel=a+'.md',before=fs.readFileSync(path.join(B,rel),'utf8');queue(rel,mapText(before,rel,'rendered-markdown'),before);
}
function mergeRanges(rs){const out=[];for(const[a,b]of rs.sort((a,b)=>a[0]-b[0]||a[1]-b[1])){if(out.length&&a<=out.at(-1)[1]+1)out.at(-1)[1]=Math.max(out.at(-1)[1],b);else out.push([a,b]);}return out;}
function rangesFromSet(set){return mergeRanges([...set].sort((a,b)=>a-b).map(x=>[x,x]));}
function missing(n,rs,excluded=[]){const all=mergeRanges([...rs.map(r=>[...r]),...excluded.map(x=>[x,x])]);let p=1,out=[];for(const[a,b]of all){if(b<p)continue;if(a>p)out.push([p,a-1]);p=Math.max(p,b+1);}if(p<=n)out.push([p,n]);return out;}
const coverageRel='evidence/read-coverage.json',coverageBefore=fs.readFileSync(path.join(B,coverageRel),'utf8'),coverage=JSON.parse(coverageBefore),coverageRows=[];
const nextFiles={...coverage.files};
for(const[k,r]of Object.entries(coverage.files)){
 if(!k.startsWith('rust:'))continue;
 const rel=k.slice(5),d=info(rel);if(r.sha256!==d.old.sha256)throw Error('Old read identity mismatch '+rel);
 const credit=new Set(),oldEx=[...(r.excluded_redacted_lines??[]),...(r.excluded_unverified_lines??[])];
 for(const[a,b]of r.ranges)for(let l=a;l<=b;l++){
  if(l>d.old.lines||oldEx.includes(l))continue;
  if(d.changes.some(c=>c.old_count>0&&l>=c.old_start&&l<c.old_start+c.old_count))continue;
  const nl=point(l,d);
  if(d.old.text_lines[l-1]!==d.active.text_lines[nl-1])throw Error('Unchanged read byte mismatch '+rel+':'+l);
  credit.add(nl);
 }
 let reviewedAdded=0;
 for(const c of d.changes){if(!approved.get(rel)?.has(key(c)))throw Error('Unreviewed credit '+rel);for(let l=c.active_start;l<c.active_start+c.active_count;l++)if(l<=d.active.lines&&!credit.has(l)){credit.add(l);reviewedAdded++;}}
 function mapExcluded(ls){const out=new Set();for(const l of ls){const n=mappedRange(rel,l,l);for(let x=n.new_range[0];x<=n.new_range[1];x++)out.add(x);}return[...out].sort((a,b)=>a-b);}
 const red=mapExcluded(r.excluded_redacted_lines??[]),unv=mapExcluded(r.excluded_unverified_lines??[]);
 for(const l of[...red,...unv])credit.delete(l);
 const ranges=rangesFromSet(credit),gaps=missing(d.active.lines,ranges,[...red,...unv]),newKey='rust:'+d.active_path;
 nextFiles[newKey]={...r,ranges,sha256:d.active.sha256,current_sha256:d.active.sha256,total_lines:d.active.lines,visible_lines:credit.size,fully_read:credit.size===d.active.lines,excluded_redacted_lines:red,excluded_unverified_lines:unv,ordinary_missing_ranges:gaps,read_credit_kind:'exact-unchanged-line-transfer-plus-reviewed-changes',hash_scope:'Active hash matches the fixed source map. Read credit consists only of byte-identical transferred old lines and explicitly reviewed changed lines, after preserving all redaction/unverified exclusions.',relocation:{old_path:rel,old_sha256:r.sha256,old_ranges:r.ranges,old_excluded_redacted_lines:r.excluded_redacted_lines??[],old_excluded_unverified_lines:r.excluded_unverified_lines??[],old_read_sources_coordinate_system:map.identities.old.head,review_artifacts:[...new Set([...approved.get(rel)?.values()??[]].map(x=>x.artifact))],changed_lines_reviewed:reviewedAdded}};
 if(newKey!==k)delete nextFiles[k];
 coverageRows.push({old_key:k,new_key:newKey,old_sha256:r.sha256,active_sha256:d.active.sha256,old_ranges:r.ranges,new_ranges:ranges,old_redacted:r.excluded_redacted_lines??[],new_redacted:red,old_unverified:r.excluded_unverified_lines??[],new_unverified:unv,ordinary_missing_ranges:gaps,reviewed_changed_lines:reviewedAdded});
}
coverage.files=nextFiles;coverage.original_source_roots=coverage.source_roots;coverage.source_roots={...coverage.source_roots,rust:activeRoot};
coverage.active_comparison={head:map.identities.active.head,root:activeRoot,method:'Exact source-map offsets for unchanged citations; reviewed changed segments for overlaps. Historical baseline_heads and input hashes are retained.'};
coverage.historical_coordinate_indexes={keys:['rust_non_crates','legacy_only_records'].filter(k=>k in coverage),rust_root:oldRoot,rust_head:map.identities.old.head,method:'These auxiliary indexes retain their original ranges and hash attribution. Only files[] is the canonical current-coordinate read-credit map; no auxiliary index creates new active read credit.'};
coverage.relocation_report='evidence/relocation-rust-remap.json';
coverage.summary={...coverage.summary,files:Object.keys(nextFiles).length,visible_lines:Object.values(nextFiles).reduce((n,r)=>n+r.visible_lines,0),fully_visible_files:Object.values(nextFiles).filter(r=>r.fully_read).length,redacted_lines:Object.values(nextFiles).reduce((n,r)=>n+(r.excluded_redacted_lines??[]).length,0),unverified_lines:Object.values(nextFiles).reduce((n,r)=>n+(r.excluded_unverified_lines??[]).length,0),active_rust_files:coverageRows.length};
queue(coverageRel,coverage,coverageBefore);
const summary={citation_occurrences:citations.length,unique_old_citations:new Set(citations.map(c=>c.old_citation)).size,methods:Object.fromEntries(['unchanged','shifted','reviewed'].map(m=>[m,citations.filter(c=>c.method===m).length])),source_files:validatedFiles.size,coverage_files:coverageRows.length,planned_files:planned.length,changed_files:planned.filter(x=>x.changed).length};
const report={schema_version:1,captured_at:new Date().toISOString(),mode,source_only:true,application_or_test_execution:false,old_root:oldRoot,active_root:activeRoot,old_head:map.identities.old.head,active_head:map.identities.active.head,source_map_sha256:sha(fs.readFileSync(path.join(E,'active-checkout-source-map.json'))),hunk_map_sha256:sha(fs.readFileSync(path.join(E,'active-checkout-diff-hunks-complete.json'))),method:'Unchanged ranges use cumulative exact offsets. Any changed-segment overlap or renamed path requires the saved manual old/active review. Read credit transfers only byte-equal unchanged lines and reviewed changed lines, then subtracts all mapped exclusions.',review_inputs:reviewInputs,summary,inputs:inputHashes,files:planned.map(({after,...r})=>r),citations,canonical_read_remap:coverageRows,source_identities:[...validatedFiles.values()].map(d=>({old_path:d.old_path,active_path:d.active_path,old_sha256:d.old.sha256,active_sha256:d.active.sha256,changes:d.changes.length,status:d.status})),historical_evidence:'Baseline, reports/excerpts, lane source reads, hash attribution, reference_normalization originals and old Git provenance are preserved. Current specs, accepted comparison surfaces, first-pass findings, native defect sources, Markdown and canonical read identity are relocated.'};
report.remapper_sha256=sha(fs.readFileSync(fileURLToPath(import.meta.url)));
report.already_active_source_identities=[...alreadyActiveFiles.values()];
report.summary.already_active_citations=citations.filter(c=>c.original_coordinate_space==='active').length;
report.summary.relocated_source_files=validatedFiles.size;
report.summary.source_files=new Set([...validatedFiles.values()].map(d=>d.active_path).concat([...alreadyActiveFiles.keys()])).size;
if(mode==='apply'){
 const preview=read('relocation-rust-remap-preview.json');
 for(const k of['old_root','active_root','old_head','active_head','source_map_sha256','hunk_map_sha256','remapper_sha256','review_inputs','inputs','files','source_identities','already_active_source_identities','summary','citations','canonical_read_remap'])if(JSON.stringify(preview[k])!==JSON.stringify(report[k]))throw Error('Fresh reviewed preview differs at '+k);
 if(fs.existsSync(path.join(E,'read-coverage-before-active-relocation.json')))throw Error('Already remapped');
 for(const f of planned)if(sha(fs.readFileSync(path.join(B,f.path)))!==f.before_sha256)throw Error('Concurrent content change '+f.path);
 fs.writeFileSync(path.join(E,'read-coverage-before-active-relocation.json'),coverageBefore);
 for(const f of planned)if(f.changed)fs.writeFileSync(path.join(B,f.path),f.after);
 const manifestList=areas.map(a=>read(a+'.json')),rows=manifestList.flatMap(m=>m.surfaces??[]),csv=rs=>rs.map(r=>r.map(v=>{const s=String(v??'');return /[",\r\n]/.test(s)?'"'+s.replaceAll('"','""')+'"':s;}).join(',')).join('\n')+'\n';
 for(const category of new Set(rows.map(r=>r.category)))fs.writeFileSync(path.join(B,'surfaces',category+'.csv'),csv([['name','ice_path:line','rust_path:line_or_blank','status','entry_id'],...rows.filter(r=>r.category===category).map(r=>[r.name,r.ice_ref,r.rust_ref??'',r.status,r.entry_id??''])]));
 fs.writeFileSync(path.join(E,'relocation-rust-remap.json'),JSON.stringify(report,null,2)+'\n');
 fs.writeFileSync(path.join(E,'active-relocation-verification.json'),JSON.stringify({captured_at:new Date().toISOString(),all_rust_citations_mapped_and_checked:true,source_only:true,old_head:report.old_head,active_head:report.active_head,summary,report:'evidence/relocation-rust-remap.json',historical_provenance_preserved:true,source_execution:false},null,2)+'\n');
}else fs.writeFileSync(path.join(E,'relocation-rust-remap-preview.json'),JSON.stringify(report,null,2)+'\n');
console.log(JSON.stringify({mode,...summary,original_contracts_unchanged:true,applied_to_active:mode==='apply',output:mode==='apply'?'evidence/relocation-rust-remap.json':'evidence/relocation-rust-remap-preview.json'},null,2));
