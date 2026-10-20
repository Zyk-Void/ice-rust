import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
if(["declarations","source"].includes(process.argv[2]))assertAuditOpen();
// Bounded coordinator inspection and read evidence for audit closeout.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {fileURLToPath} from 'node:url';
const B=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..'), E=path.join(B,'evidence');
const areas=['ai','agent','tools','session','settings','extensions','rpc-remote','tui','subagents','safe-verify','observability','cognee','blackhole','providers-auth','cli-packaging','misc'];
const read=n=>JSON.parse(fs.readFileSync(path.join(E,n),'utf8'));
const [mode,...args]=process.argv.slice(2);
if(mode==='declarations'){
 const q=read('qa-unnamed-exports-2026-10-20.json'),records=[],selected=args;
 const entries=areas.flatMap(area=>read('spec-'+area+'.json').entries.map(e=>({area,...e})));
 for(const area of selected){
  for(const r of q.symbols[area]){
   const m=r.ts.match(/^ice:(.+):(\d+)$/),rel=m[1],a=+m[2],p=path.join('/home/mewtwo/Zks/ice',rel);
   const raw=fs.readFileSync(p),lines=raw.toString('utf8').split('\n'),b=Math.min(lines.length,a+11);
   records.push({...r,area,path:'ice:'+rel,sha256:crypto.createHash('sha256').update(raw).digest('hex'),ranges:[[a,b]],excerpt:lines.slice(a-1,b).map((s,i)=>(a+i)+' | '+s).join('\n')});
  }
 }
 fs.writeFileSync(path.join(E,'lane-root-declarations-'+selected.join('-')+'.json'),JSON.stringify({owner:'/root',source_only:true,records},null,2)+'\n');
 const ids=new Set(records.flatMap(r=>r.covering_entries));
 console.log(JSON.stringify({declarations:records.map(({sha256,path,ranges,...r})=>r),entry_summaries:entries.filter(e=>ids.has(e.id)).map(e=>({area:e.area,id:e.id,title:e.title,what:e.what}))},null,2));
}else if(mode==='exports'){
 const q=read('qa-unnamed-exports-2026-10-20.json');
 for(const area of args)console.log(JSON.stringify({area,records:q.symbols[area]},null,2));
}else if(mode==='entries'){
 const ids=new Set(args);
 for(const area of areas)for(const e of read('spec-'+area+'.json').entries)if(ids.has(e.id))console.log(JSON.stringify({area,...e},null,2));
}else if(mode==='entry-fields'){
 const [fields,...ids]=args;
 for(const area of areas)for(const e of read('spec-'+area+'.json').entries)if(ids.includes(e.id))console.log(JSON.stringify(Object.fromEntries(['id','title',...fields.split(',')].map(k=>[k,e[k]])),null,2));
}else if(mode==='file-entries'){
 for(const area of areas)for(const e of read('spec-'+area+'.json').entries)if(args.some(p=>e.source.includes(p)))console.log(JSON.stringify({area,id:e.id,title:e.title,source:e.source,what:e.what,mechanism:e.mechanism,interfaces:e.interfaces},null,2));
}else if(mode==='source'){
 const roots={ice:'/home/mewtwo/Zks/ice',rust:path.resolve(B,'../..'),active:'/home/mewtwo/Zykairotis/ice-rust'};
 const artifact=path.join(E,'lane-root-source-review.json');
 const old=fs.existsSync(artifact)?read('lane-root-source-review.json'):{owner:'/root',source_only:true,records:[]};
 for(const ref of args){
  const m=ref.match(/^(ice|rust|active):(.+):(\d+)-(\d+)$/);if(!m)throw Error('Explicit source range required');
  const [,side,rel]=m,a=+m[3],b=+m[4],p=path.resolve(roots[side],rel);
  if(!p.startsWith(roots[side]+path.sep)||/\/(?:\.ice|\.icers)(?:\/|$)/.test(p))throw Error('Forbidden source path');
  const raw=fs.readFileSync(p),lines=raw.toString('utf8').split('\n');
  if(a<1||b<a||b>lines.length)throw Error('Out of range '+ref);
  const excerpt=lines.slice(a-1,b).map((l,i)=>(a+i)+' | '+l).join('\n');
  const record={path:side+':'+rel,sha256:crypto.createHash('sha256').update(raw).digest('hex'),ranges:[[a,b]],captured_at:new Date().toISOString(),excerpt};
  old.records.push(record);
  console.log(JSON.stringify(record,null,2));
 }
 fs.writeFileSync(artifact,JSON.stringify(old,null,2)+'\n');
}else if(mode==='shape'){
 for(const n of args){const x=read(n);console.log(JSON.stringify({name:n,shape:Array.isArray(x)?{length:x.length,sample:x.slice(0,2)}:Object.fromEntries(Object.entries(x).map(([k,v])=>[k,Array.isArray(v)?{length:v.length,sample:v.slice(0,1)}:typeof v==='object'&&v?{keys:Object.keys(v),value:JSON.stringify(v).length<1500?v:undefined}:v]))},null,2));}
}else throw Error('Unknown mode');
