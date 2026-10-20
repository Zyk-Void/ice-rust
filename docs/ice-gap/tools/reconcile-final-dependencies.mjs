// Apply the coordinator-reviewed dependency decisions. Audit metadata only.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {fileURLToPath} from 'node:url';
const base=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const E=path.join(base,'evidence');
const areas=['ai','agent','tools','session','settings','extensions','rpc-remote','tui','subagents','safe-verify','observability','cognee','blackhole','providers-auth','cli-packaging','misc'];
const mode=process.argv[2]??'preview';
if(!['preview','apply'].includes(mode))throw Error('preview or apply required');
const read=n=>JSON.parse(fs.readFileSync(path.join(E,n),'utf8'));
const hash=n=>crypto.createHash('sha256').update(fs.readFileSync(path.join(E,n))).digest('hex');
const draft=read('coordinator-dependency-resolutions-draft.json');
const decisions=read('dependency-coordinator-decisions.json');
if(hash('coordinator-dependency-resolutions-draft.json')!==decisions.reviewed_draft_sha256)throw Error('Reviewed dependency draft changed');
const semantic=read('qa-semantic-dependency-repair-slice.json');
if(hash(semantic.source)!==semantic.source_sha256)throw Error('Independent semantic review changed');
const docs=areas.map(area=>({area,spec:read('spec-'+area+'.json'),manifest:fs.existsSync(path.join(E,area+'.json'))?read(area+'.json'):null}));
const all=new Map(docs.flatMap(d=>d.spec.entries.map(e=>[e.id,{area:d.area,entry:e}])));
if(all.size!==docs.reduce((n,d)=>n+d.spec.entries.length,0))throw Error('Duplicate entry IDs');
const before=[],edges=new Map(),resolved=[],problems=[];
const edgeKey=(a,b)=>a+'|'+b;
function relation(a,b,kind,reason,review,force=true){
 if(!all.has(a)||!all.has(b)){problems.push({kind:'unknown_id',from:a,to:b,review});return;}
 if(a===b){problems.push({kind:'self_dependency',from:a,review});return;}
 const k=edgeKey(a,b),old=edges.get(k);
 if(old&&!force){old.reviews.push(review);return;}
 edges.set(k,{from:a,to:b,kind,reason,reviews:[...new Set([...(old?.reviews??[]),review])]});
}
for(const [id,{area,entry:e}]of all){
 before.push({area,id,dependencies:[...(e.dependencies??[])],dependency_text:e.dependency_text??''});
 for(const target of e.dependencies??[])relation(id,target,'prerequisite','Previously authored entry dependency, preserved unless explicitly reclassified by the source-based review.','original-entry');
}
const perRequest=new Map();
function requestKey(area,id,target,capability){return JSON.stringify([area,id,target,capability]);}
draft.records.forEach((r,index)=>{
 const v={...(r.resolution??{}),...(decisions.group_overrides[String(index)]??{})};
 if(!v.kind||!Array.isArray(v.entry_ids))throw Error('Unresolved root group '+index);
 for(const from of r.from){
  const narrow=decisions.grouped_prerequisites[String(index)]?.[from]??decisions.grouped_relationships[String(index)]?.[from];
  if(r.from.length>1&&v.entry_ids.length>1&&!narrow)throw Error('Grouped mapping lacks a per-entry resolution '+index+' '+from);
  const targets=narrow??v.entry_ids;
  const record={review_id:'ROOTDEP-'+String(index+1).padStart(3,'0'),area:r.area,entry_id:from,target_area:r.target_area,capability:r.capability,kind:v.kind,entry_ids:targets,rationale:v.rationale,previous_confirmed_ids:r.previous_confirmed_ids??[]};
  perRequest.set(requestKey(r.area,from,r.target_area,r.capability),record);resolved.push(record);
  for(const target of targets)relation(from,target,v.kind,v.rationale,record.review_id);
 }
});
for(const r of semantic.resolutions){
 const add=r.review_id==='DEPQA-031'?['AI-019']:(r.add_prerequisite_ids??[]);
 const disposition=r.review_id==='DEPQA-031'?'resolved_prerequisite_contracts':r.disposition??'resolved_prerequisite_contracts';
 const record={review_id:r.review_id,area:r.area??all.get(r.entry_id)?.area,entry_id:r.entry_id,target_area:r.original_request?.area,capability:r.original_request?.capability,kind:disposition==='external_producer_unverified'?'external-unverified':add.length?'prerequisite':'shared-contract',entry_ids:add,related_entry_ids:r.related_entry_ids??[],rationale:r.review_id==='DEPQA-031'?'AI-019 explicitly owns the complete nested provider data contracts; AI-006 remains request-option authority.':r.rationale??'Reviewed coordinator follow-up.',disposition};
 if(r.original_request)perRequest.set(requestKey(record.area,r.entry_id,r.original_request.area,r.original_request.capability),record);
 resolved.push(record);
 for(const target of add)relation(r.entry_id,target,'prerequisite',record.rationale,r.review_id);
 for(const target of r.related_entry_ids??[])relation(r.entry_id,target,'shared-contract',record.rationale,r.review_id);
 if(r.relocate_prerequisites)for(const target of r.relocate_prerequisites.add_prerequisite_ids)relation(r.relocate_prerequisites.entry_id,target,'prerequisite',r.relocate_prerequisites.reason,r.review_id);
}
for(const [entry,targets]of Object.entries(decisions.additional_prerequisites??{}))for(const target of targets)relation(entry,target,'prerequisite','Additional exact contract dependency confirmed during coordinator closeout.','coordinator-additional');
for(const r of decisions.relation_overrides??[])relation(r.entry_id,r.target_id,r.kind,r.reason,'coordinator-relationship-override');
const metadata=[];
for(const d of docs){
 if(!d.manifest){problems.push({kind:'missing_accepted_manifest',area:d.area});continue;}
 const rs=[];
 for(const request of d.manifest.dependency_requests??[]){
  const key=requestKey(d.area,request.entry_id,request.area,request.capability);
  const r=perRequest.get(key);
  if(!r){problems.push({kind:'unresolved_request',area:d.area,request});continue;}
  rs.push({...request,status:r.kind==='external-unverified'?'externally-unverified-with-reason':'resolved',resolved_entry_ids:r.entry_ids,related_entry_ids:r.related_entry_ids??[],relation:r.kind,review_id:r.review_id,rationale:r.rationale,previous_confirmed_entry_ids:request.confirmed_entry_ids??request.entry_ids??[]});
 }
 metadata.push({area:d.area,resolutions:rs,original_requests:d.manifest.dependency_requests??[]});
}
for(const [id,{entry:e}]of all){
 const ee=[...edges.values()].filter(x=>x.from===id);
 e.dependencies=ee.filter(x=>x.kind==='prerequisite').map(x=>x.to).sort();
 e.dependency_relations=ee.map(({to,kind,reason,reviews})=>({entry_id:to,kind,reason,reviews})).sort((a,b)=>a.entry_id.localeCompare(b.entry_id));
 const extras=ee.filter(x=>x.kind!=='prerequisite');
 if(extras.length){
  const groups=['consumer','shared-contract'].flatMap(kind=>{const a=extras.filter(x=>x.kind===kind);return a.length?[(kind==='consumer'?'Consuming integrations':'Shared contract references')+': '+a.map(x=>'\x60'+x.to+'\x60').join(', ')+'.']:[];});
  e.dependency_text=(e.dependency_text?e.dependency_text.trim()+'\n\n':'')+'Final dependency classification. '+groups.join(' ')+' These links are retained in the typed graph and do not add a prerequisite edge.';
 }
}
const changed=before.filter(b=>JSON.stringify(all.get(b.id).entry.dependencies)!==JSON.stringify(b.dependencies)).map(b=>({...b,after:all.get(b.id).entry.dependencies}));
const report={schema_version:1,captured_at:new Date().toISOString(),owner:'/root',mode,source_only:true,inputs:['coordinator-dependency-resolutions-draft.json','dependency-coordinator-decisions.json','qa-semantic-dependency-repair-slice.json','qa-semantic-dependency-suggestions.json','ai-schema-dependency-amendment.json'].map(name=>({name,sha256:hash(name)})),root_groups:draft.records.length,independent_resolutions:semantic.resolutions.length,request_resolutions:metadata,decisions:resolved,changed_dependencies:changed,edges:[...edges.values()].sort((a,b)=>a.from.localeCompare(b.from)||a.to.localeCompare(b.to)),problems,external_unverified:resolved.filter(r=>r.kind==='external-unverified'),summary:{entries:all.size,resolved_requests:metadata.reduce((n,m)=>n+m.resolutions.length,0),changed_entries:changed.length,edges:edges.size,edge_kinds:Object.fromEntries(['prerequisite','consumer','shared-contract'].map(k=>[k,[...edges.values()].filter(e=>e.kind===k).length])),unresolved_requests:problems.filter(p=>p.kind==='unresolved_request').length}};
if(mode==='apply'){
 if(problems.length)throw Error(JSON.stringify(problems));
 if(read('progress.json').complete)throw Error('Completed audit protected from historical mutation');
 if(fs.existsSync(path.join(E,'dependency-final-reconciliation.json')))throw Error('Reconciliation already applied');
 for(const d of docs){
  fs.writeFileSync(path.join(E,'spec-'+d.area+'.json'),JSON.stringify(d.spec,null,2)+'\n');
  for(const name of[d.area+'.json','proposal-'+d.area+'.json']){
   if(!fs.existsSync(path.join(E,name)))continue;
   const m=read(name),mr=metadata.find(r=>r.area===d.area);
   m.entries=m.entries.map(old=>{const e=all.get(old.id).entry;return {...old,dependencies:e.dependencies,dependency_relations:e.dependency_relations};});
   m.dependency_request_history=mr.original_requests;
   m.dependency_resolutions=mr.resolutions;
   m.dependency_requests=[];
   fs.writeFileSync(path.join(E,name),JSON.stringify(m,null,2)+'\n');
  }
 }
 const defects=read('rust-defects.json'),q=defects.find(d=>d.id==='RUST-068');
 if(!q)throw Error('RUST-068 missing');
 q.provenance=decisions.native_queue_provenance.provenance;
 q.provenance_status=decisions.native_queue_provenance.provenance_status;
 q.provenance_review={owner:'/root',evidence:'evidence/dependency-coordinator-decisions.json',reviewed_targets:decisions.native_queue_provenance.reviewed_targets};
 fs.writeFileSync(path.join(E,'rust-defects.json'),JSON.stringify(defects,null,2)+'\n');
 fs.writeFileSync(path.join(E,'dependency-final-reconciliation.json'),JSON.stringify(report,null,2)+'\n');
}
console.log(JSON.stringify({...report,inputs:undefined,request_resolutions:undefined,decisions:undefined,changed_dependencies:changed.slice(0,4),edges:undefined},null,2));
if(problems.length)process.exitCode=1;
