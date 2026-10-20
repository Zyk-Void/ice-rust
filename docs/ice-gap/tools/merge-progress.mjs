import { auditHelperBase } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
assertAuditOpen();
// Merge only explicitly authored area evidence. Never fill pending work with fabricated mappings.
import fs from 'node:fs';
import path from 'node:path';
const base=auditHelperBase;
const areaNames=['ai','agent','tools','session','settings','extensions','rpc-remote','tui','subagents','safe-verify','observability','cognee','blackhole','providers-auth','cli-packaging','misc'];
const csv=rows=>rows.map(row=>row.map(v=>{const s=String(v??'');return /[",\r\n]/.test(s)?'"'+s.replaceAll('"','""')+'"':s;}).join(',')).join('\n')+'\n';
const ledger=JSON.parse(fs.readFileSync(path.join(base,'evidence/tracked-files.json'),'utf8'));
const candidates=JSON.parse(fs.readFileSync(path.join(base,'evidence/surface-candidates.json'),'utf8'));
const areas=[];
for(const a of areaNames){const f=path.join(base,'evidence',a+'.json');if(fs.existsSync(f))areas.push(JSON.parse(fs.readFileSync(f,'utf8')));}
const mappings=new Map();
for(const a of areas)for(const r of a.ledger){if(mappings.has(r.path))throw Error('Duplicate ledger ownership '+r.path);mappings.set(r.path,r);}
for(const r of ledger){const m=mappings.get(r.path);if(m){r.entry_ids=m.entry_ids.join(';');r.status=m.status;r.notes=m.notes;}}
for(const a of areas)for(const r of a.supplemental_ledger??[]){if(ledger.some(x=>x.path===r.path))continue;ledger.push(r);}
fs.writeFileSync(path.join(base,'ledger.csv'),csv([['path','lines','area','entry_ids','status','notes'],...ledger.map(r=>[r.path,r.lines,r.area,Array.isArray(r.entry_ids)?r.entry_ids.join(';'):r.entry_ids,r.status,r.notes])]));
const rows=areas.flatMap(a=>a.surfaces??[]);
const done=new Set(areas.flatMap(a=>[...(a.surfaces??[]).flatMap(s=>s.candidate_ids??[]),...(a.candidate_exclusions??[]).map(c=>c.candidate_id)]));
for(const c of candidates)if(!done.has(c.candidate_id))rows.push({category:c.category,name:c.name,ice_ref:c.path+':'+c.line,rust_ref:'',status:'pending',entry_id:''});
const categories=[...new Set([...candidates.map(c=>c.category),...rows.map(r=>r.category)])].sort();
for(const c of categories)fs.writeFileSync(path.join(base,'surfaces',c+'.csv'),csv([['name','ice_path:line','rust_path:line_or_blank','status','entry_id'],...rows.filter(r=>r.category===c).map(r=>[r.name,r.ice_ref,r.rust_ref??'',r.status,r.entry_id??''])]));
const progress={captured_at:new Date().toISOString(),complete:false,ledger_rows:ledger.length,mapped:ledger.filter(r=>r.status==='mapped').length,not_applicable:ledger.filter(r=>r.status==='not-applicable').length,pending:ledger.filter(r=>r.status==='pending').length,areas:areas.map(a=>({area:a.area,complete:a.complete,entries:a.entries.length})),surface_counts:Object.fromEntries(categories.map(c=>[c,{total:rows.filter(r=>r.category===c).length,pending:rows.filter(r=>r.category===c&&r.status==='pending').length}])),entry_count:areas.reduce((n,a)=>n+a.entries.length,0),first_pass_claims:areas.reduce((n,a)=>n+(a.claims??[]).length,0)};
fs.writeFileSync(path.join(base,'evidence/progress.json'),JSON.stringify(progress,null,2)+'\n');
console.log(JSON.stringify(progress,null,2));
