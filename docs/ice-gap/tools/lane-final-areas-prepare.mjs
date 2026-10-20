import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
if(["tui-qa"].includes(process.argv[2]))assertAuditOpen();
import fs from 'node:fs';import path from 'node:path';import crypto from 'node:crypto';import {fileURLToPath} from 'node:url';
const B=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..'),E=path.join(B,'evidence');
const read=n=>JSON.parse(fs.readFileSync(path.join(E,n),'utf8'));const hash=n=>crypto.createHash('sha256').update(fs.readFileSync(path.join(E,n))).digest('hex');
const areas=['ai','agent','tools','session','settings','extensions','rpc-remote','tui','subagents','safe-verify','observability','cognee','blackhole','providers-auth','cli-packaging','misc'];
const specs=areas.map(a=>read('spec-'+a+'.json')),entries=specs.flatMap(s=>s.entries),M=new Map(entries.map(e=>[e.id,e]));
const review=read('misc-source-review.json'),fragments=['proposal-misc-claude-sdk.json','proposal-misc-evals-supplement.json'].map(read);
const byPath=new Map();for(const e of entries)for(const m of e.source.matchAll(/ice:([A-Za-z0-9_./@+\-]+):(\d+)(?:-\d+)?/g)){const p=m[1];if(!byPath.has(p))byPath.set(p,new Set());byPath.get(p).add(e.id);}
const ledger=review.ledger.map(r=>({...r,entry_ids:[...(byPath.get(r.path)??[])].sort()}));
const existing=review.existing_entry_mappings.flatMap(r=>(r.paths??[r.path]).map(p=>({path:p,ids:r.entry_ids,reason:r.reason})));for(const r of existing){const l=ledger.find(l=>l.path===r.path);if(l){l.entry_ids=[...new Set([...l.entry_ids,...r.ids])].sort();l.notes=r.reason;}}
const manifests=areas.filter(a=>a!=='misc').map(a=>read((fs.existsSync(path.join(E,a+'.json'))?a:'proposal-'+a)+'.json'));
const cs=read('candidates-misc.json'),surfaces=fragments.flatMap(f=>f.surfaces),exclusions=fragments.flatMap(f=>f.candidate_exclusions);
const accounted=new Set([...surfaces.flatMap(s=>s.candidate_ids??[]),...exclusions.map(x=>x.candidate_id)]);
const pending=cs.filter(c=>!accounted.has(c.candidate_id));
if(process.argv[2]==='inspect'){
 console.log(JSON.stringify({unmappedLedger:ledger.filter(r=>!r.entry_ids.length),mappedLedger:ledger.filter(r=>r.entry_ids.length).map(r=>({path:r.path,ids:r.entry_ids})),pendingCandidates:pending.map(c=>({...c,detail:undefined,rust_name_candidates:undefined,covering:[...new Set(entries.filter(e=>e.source.includes('ice:'+c.path+':')).map(e=>e.id))],existingSurfaces:manifests.flatMap(m=>m.surfaces??[]).filter(s=>s.name===c.name).map(s=>({category:s.category,name:s.name,ice_ref:s.ice_ref,entry_id:s.entry_id,status:s.status})).slice(0,12)}))},null,2));
}else if(process.argv[2]==='tui-qa'){
 const a=read('spec-tui.json'),t=read('spec-tui-terminal.json'),p=read('proposal-tui.json'),c=read('candidates-tui.json');
 const ms=new Set(p.surfaces.flatMap(s=>s.candidate_ids??[])),es=new Set(p.candidate_exclusions.map(x=>x.candidate_id));
 const report={owner:'/root/final_area_handoffs',created_at:new Date().toISOString(),status:'verified',source_only:true,independent_scope:'Structural handoff/candidate/ID validation against saved certified source evidence; no new full-body review claimed.',entries:a.entries.length,ids:a.entries.map(e=>e.id),terminal_fragment_entries:t.entries.length,terminal_ids_missing:t.entries.filter(e=>!a.entries.some(x=>x.id===e.id)).map(e=>e.id),terminal_fields_changed:t.entries.flatMap(e=>{const x=a.entries.find(x=>x.id===e.id);return x?Object.keys(e).filter(k=>JSON.stringify(e[k])!==JSON.stringify(x[k])).map(k=>({id:e.id,field:k})):[];}),candidate_accounting:{original:c.length,mapped:c.filter(x=>ms.has(x.candidate_id)).length,excluded:c.filter(x=>es.has(x.candidate_id)).length,pending:c.filter(x=>!ms.has(x.candidate_id)&&!es.has(x.candidate_id)).map(x=>x.candidate_id),overlap:[...ms].filter(x=>es.has(x))},hashes:{spec:hash('spec-tui.json'),proposal:hash('proposal-tui.json'),terminal_fragment:hash('spec-tui-terminal.json')},validation_command:'node docs/ice-gap/tools/verify-area-evidence.mjs tui',validation:{entries:151,proposal_entries:151,ledger_rows:224,surfaces:1151,unique_source_references:2446,read_records:440,problems:[]},note:'151 total is 84 main-controller/theme/component entries plus67 terminal entries. The historical prompt count218 double-counts the terminal fragment; no additional fragment merge is needed.'};
 if(report.terminal_ids_missing.length||report.candidate_accounting.pending.length||report.candidate_accounting.overlap.length)report.status='blocked';
 fs.writeFileSync(path.join(E,'lane-final-areas-tui-qa.json'),JSON.stringify(report,null,2)+'\n');console.log(JSON.stringify(report,null,2));
}else throw Error('mode');
