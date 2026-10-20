// Read-only structural reconciliation across authoritative area drafts and publications.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {fileURLToPath} from 'node:url';
const base=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const areas=['ai','agent','tools','session','settings','extensions','rpc-remote','tui','subagents','safe-verify','observability','cognee','blackhole','providers-auth','cli-packaging','misc'];
const read=(n)=>JSON.parse(fs.readFileSync(path.join(base,'evidence',n),'utf8'));
const loaded=areas.flatMap(area=>{const p=path.join(base,'evidence','spec-'+area+'.json');return fs.existsSync(p)?[{area,spec:read('spec-'+area+'.json'),sha256:crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex')}]:[];});
const ids=new Set(),duplicates=[];for(const a of loaded)for(const e of a.spec.entries){if(ids.has(e.id))duplicates.push(e.id);ids.add(e.id);}
const fields=['id','title','status','source','what','mechanism','interfaces','dependencies','rust','port','size_min','size_max','priority','priority_reason'];
const report={captured_at:new Date().toISOString(),areas:[],duplicate_ids:duplicates,missing_fields:[],invalid_values:[],unknown_dependencies:[],unknown_metadata_ids:[],citation_shorthand:[],missing_native_anchors:[],published_missing_ids:[]};
const walk=(v,loc,out)=>{if(Array.isArray(v)){v.forEach((x,i)=>walk(x,loc+'['+i+']',out));return;}if(v&&typeof v==='object'){for(const[k,x]of Object.entries(v)){if(['entry_ids','entries','confirmed_entry_ids','resolved_entry_ids'].includes(k)&&Array.isArray(x)&&x.every(z=>typeof z==='string'))for(const id of x)if(/^[A-Z]+-\d{3}$/.test(id)&&!ids.has(id))out.push({location:loc+'.'+k,id});else{};if(k!=='read_evidence')walk(x,loc+'.'+k,out);}}};
for(const {area,spec,sha256}of loaded){
 const statuses={},priorities={};let text='';try{text=fs.readFileSync(path.join(base,area+'.md'),'utf8')}catch{}
 for(const e of spec.entries){
  for(const f of fields)if(!(f in e)||(typeof e[f]==='string'&&!e[f].trim()))report.missing_fields.push({area,id:e.id,field:f});
  if(!['missing','partial','different-by-design'].includes(e.status)||!/^P[0-3]$/.test(e.priority)||!Number.isFinite(e.size_min)||!Number.isFinite(e.size_max)||e.size_min<0||e.size_max<e.size_min)report.invalid_values.push({area,id:e.id});
  statuses[e.status]=(statuses[e.status]||0)+1;priorities[e.priority]=(priorities[e.priority]||0)+1;
  for(const d of e.dependencies||[])if(!ids.has(d))report.unknown_dependencies.push({area,id:e.id,dependency:d});
  if(!/(?:rust:)?crates\/[^\s\x60]+:\d+/.test(e.rust||''))report.missing_native_anchors.push({area,id:e.id,title:e.title});
  for(const[k,v]of Object.entries(e))if(typeof v==='string')for(const m of v.matchAll(/(?:ice|rust):[^\s\x60;]+:\d+(?:[-–]\d+)?(?:,\d+(?:[-–]\d+)?)+/g))report.citation_shorthand.push({area,id:e.id,field:k,value:m[0]});
  if(text&&!text.includes(e.id))report.published_missing_ids.push({area,id:e.id});
 }
 const accepted=fs.existsSync(path.join(base,'evidence',area+'.json'));
 if(accepted){const m=read(area+'.json');walk(m,'evidence/'+area+'.json',report.unknown_metadata_ids);}
 report.areas.push({area,entries:spec.entries.length,accepted,published:!!text,sha256,statuses,priorities});
}
console.log(JSON.stringify(report,null,2));
