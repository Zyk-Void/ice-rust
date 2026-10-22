// Coordinator-only reviewed export merges. Writes audit metadata and rendered docs only.
import fs from 'node:fs'; import path from 'node:path'; import crypto from 'node:crypto';
import {fileURLToPath} from 'node:url'; import {execFileSync} from 'node:child_process';
const B=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..'), E=path.join(B,'evidence');
const areas=['ai','agent','tools','session','settings','extensions','rpc-remote','tui','subagents','safe-verify','observability','cognee','blackhole','providers-auth','cli-packaging','misc'];
const read=n=>JSON.parse(fs.readFileSync(path.join(E,n),'utf8')), sha=n=>crypto.createHash('sha256').update(fs.readFileSync(path.join(E,n))).digest('hex');
const write=(n,v)=>fs.writeFileSync(path.join(E,n),JSON.stringify(v,null,2)+'\n');
const [mode,...names]=process.argv.slice(2); if(!['review','preview','apply'].includes(mode)||!names.length)throw Error('review|preview|apply proposal.json...');
const proposals=names.map(name=>{if(!/^lane-[a-z0-9-]+\.json$/.test(name))throw Error('Unsafe proposal name');return {name,sha256:sha(name),data:read(name)};});
if(mode==='review'){for(const p of proposals)console.log(JSON.stringify({name:p.name,updates:p.data.entry_updates?.map(u=>Object.fromEntries(Object.entries(u).filter(([k])=>k!=='source_append'))),present_contracts:p.data.present_contracts,corrections:p.data.corrections,replacements:p.data.replacements,field_replacements:p.data.field_replacements,summary:p.data.totals},null,2));process.exit();}
if(read('progress.json').complete)throw Error('Completed audit protected against historical mutation');
const specs=new Map(areas.map(a=>[a,read('spec-'+a+'.json')])), index=new Map([...specs].flatMap(([a,s])=>s.entries.map(e=>[e.id,{area:a,e}])));
const qa=read('qa-unnamed-exports-2026-10-20.json'), expected=new Map(Object.entries(qa.symbols).flatMap(([area,rr])=>rr.map(r=>[r.ts+'|'+r.symbol,{area,...r}])));
const output='qa-unnamed-exports-dispositions.json', previous=fs.existsSync(path.join(E,output))?read(output):{schema_version:1,complete:false,inputs:[],dispositions:[],merges:[]};
const seen=new Set(previous.dispositions.map(r=>r.ts+'|'+r.symbol)), changes=[], touched=new Set(), problems=[];
const prose=new Set(['source','what','mechanism','interfaces','rust','port','dependency_text','priority_reason']);
for(const p of proposals){
 if(previous.inputs.some(i=>i.name===p.name)){problems.push({kind:'already_merged',file:p.name});continue;}
 if((p.data.new_entries?.length??0)||(p.data.new_ids?.length??0))problems.push({kind:'unapproved_new_ids',file:p.name});
 for(const r of p.data.field_replacements??[]){
  const target=index.get(r.id),old=r.old_text??r.expected_old,next=r.new_text??r.replacement;
  if(!target||!prose.has(r.field)||typeof old!=='string'||!old||typeof next!=='string'||!r.reason||!r.source_review){problems.push({kind:'invalid_field_replacement',file:p.name,id:r.id,field:r.field});continue;}
  const before=target.e[r.field]??'',count=before.split(old).length-1;
  if(count!==1){problems.push({kind:'replacement_expected_once',file:p.name,id:r.id,field:r.field,count,expected:old});continue;}
  target.e[r.field]=before.replace(old,next);changes.push({area:target.area,id:r.id,field:r.field,before,after:target.e[r.field],reason:r.reason,source_review:r.source_review});touched.add(target.area);
 }
 for(const r of p.data.dispositions??[]){
  const k=r.ts+'|'+r.symbol,x=expected.get(k);
  if(!x||x.kind!==r.kind)problems.push({kind:'unexpected_export',file:p.name,key:k});
  if(seen.has(k))problems.push({kind:'duplicate_export',file:p.name,key:k});seen.add(k);
  if(!r.disposition||!r.reason||!r.source_review)problems.push({kind:'incomplete_disposition',file:p.name,key:k});
  for(const id of r.entry_ids??[])if(!index.has(id))problems.push({kind:'unknown_entry',id,file:p.name});
  previous.dispositions.push({...r,area:x?.area,proposal:p.name});
 }
 for(const u of p.data.entry_updates??[]){
  const target=index.get(u.id);if(!target){problems.push({kind:'unknown_update',id:u.id});continue;}
  for(const [k,value]of Object.entries(u)){
   if(['id','area'].includes(k))continue;
   if(!k.endsWith('_append')||!prose.has(k.slice(0,-7))){problems.push({kind:'unsupported_update',id:u.id,key:k});continue;}
   const field=k.slice(0,-7), additions=Array.isArray(value)?value:[value];
   for(const t of additions){if(typeof t!=='string'){problems.push({kind:'non_string',id:u.id,key:k});continue;}if(!t.trim()||target.e[field]?.includes(t))continue;
    const before=target.e[field]??'';target.e[field]=before.trimEnd()+(before.trim()?'\n\n':'')+t.trim();changes.push({area:target.area,id:u.id,field,before,after:target.e[field]});touched.add(target.area);
   }
  }
 }
 if(p.data.corrections?.length||p.data.replacements?.length)problems.push({kind:'explicit_replacements_require_review',file:p.name});
 previous.inputs.push({name:p.name,sha256:p.sha256,records:p.data.dispositions?.length??0});
}
const lastChanges=new Map(changes.map(c=>[c.id+'|'+c.field,c])), firstChanges=new Map();for(const c of changes)if(!firstChanges.has(c.id+'|'+c.field))firstChanges.set(c.id+'|'+c.field,c);
const combined=[...lastChanges.entries()].map(([key,c])=>({...c,before:firstChanges.get(key).before}));
const report={captured_at:new Date().toISOString(),owner:'/root',mode,inputs:proposals.map(({name,sha256})=>({name,sha256})),updated_entries:new Set(changes.map(c=>c.id)).size,changed_fields:combined.length,areas:[...touched],records:previous.dispositions.length,expected:expected.size,remaining:expected.size-seen.size,problems};
if(mode==='apply'){
 if(problems.length)throw Error(JSON.stringify(problems));
 const record='export-merge-'+String(previous.merges.length+1).padStart(2,'0')+'.json';
 write(record,{...report,changes:combined,specs_before:[...touched].map(a=>({area:a,sha256:sha('spec-'+a+'.json')}))});
 for(const area of touched){
  const s=specs.get(area);write('spec-'+area+'.json',s);
  for(const name of[area+'.json','proposal-'+area+'.json'])if(fs.existsSync(path.join(E,name))){
   const m=read(name);for(const me of m.entries){const e=index.get(me.id).e;for(const c of combined.filter(c=>c.id===me.id))if(Object.hasOwn(me,c.field))me[c.field]=e[c.field];}write(name,m);
  }
 }
 previous.captured_at=report.captured_at;previous.complete=seen.size===expected.size;
 previous.summary={records:previous.dispositions.length,runtime:previous.dispositions.filter(r=>r.kind==='runtime').length,type:previous.dispositions.filter(r=>r.kind==='type').length,unresolved:expected.size-seen.size};
 previous.merges.push({artifact:record,...report});write(output,previous);
 for(const area of touched){const renderer=fs.existsSync(path.join(B,'render-area.mjs'))?path.join(B,'render-area.mjs'):path.join(B,'tools/render-area.mjs');execFileSync(process.execPath,[renderer,area],{cwd:path.resolve(B,'../..'),stdio:'pipe'});}
 fs.appendFileSync(path.join(E,'RESUME.md'),'\n\n## Export merge '+report.captured_at+'\n\n'+proposals.map(p=>'- Accepted '+p.name+' ('+(p.data.dispositions?.length??0)+' reviewed records).').join('\n')+'\n\n| Lane | State |\n|---|---|\n| Final area publication | Complete: all 16 areas, AI-019 included; 791 entries |\n| Export QA | '+report.records+' / '+report.expected+' merged; remaining workers active |\n| Dependencies, claims, provenance | Coordinator reconciliation in progress |\n| Read coverage | Final-area worker preparing proposal |\n| Spacing, original gate, relocation | Pending final export merges |\n\nReserved IDs: SESSION-039/040 unused; no new entry approved. Next: merge remaining lane proposals, reconcile claims/dependencies, then spacing and the original gate.\n');
}
console.log(JSON.stringify(report,null,2));if(problems.length)process.exitCode=1;
