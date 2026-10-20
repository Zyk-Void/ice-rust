import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
if(["batch"].includes(process.argv[2]))assertAuditOpen();
// Lane-scoped read-only export evidence helper. Does not edit shared audit files or source.
import fs from 'node:fs'; import path from 'node:path'; import crypto from 'node:crypto'; import {fileURLToPath} from 'node:url';
const base=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..'), E=path.join(base,'evidence'), TS='/home/mewtwo/Zks/ice';
const areas=['rpc-remote','ai','providers-auth','extensions'];
const [mode,area,...args]=process.argv.slice(2);
if(!areas.includes(area))throw Error('Lane area required');
const qa=JSON.parse(fs.readFileSync(path.join(E,'qa-unnamed-exports-2026-10-20.json'),'utf8')).symbols[area];
const spec=JSON.parse(fs.readFileSync(path.join(E,'spec-'+area+'.json'),'utf8'));
const extra=area==='ai'?JSON.parse(fs.readFileSync(path.join(E,'spec-ai-schema-supplement.json'),'utf8')).entries:[];
const entries=[...spec.entries,...extra.filter(e=>!spec.entries.some(x=>x.id===e.id))];
const sourceCache=new Map();
function source(p){if(!p.startsWith('packages/')||p.includes('..'))throw Error('Invalid source path');if(!sourceCache.has(p)){const raw=fs.readFileSync(path.join(TS,p));sourceCache.set(p,{lines:raw.toString().split('\n'),sha256:crypto.createHash('sha256').update(raw).digest('hex')});}return sourceCache.get(p);}
if(mode==='contracts'){for(const e of entries)if(!args.length||args.includes(e.id)){const {id,source,what,mechanism,interfaces}=e;console.log(JSON.stringify({id,source,what,mechanism,interfaces}));}}
else if(mode==='inventory'){
const groups={};for(const q of qa){const m=q.ts.match(/^ice:(.*):(\d+)$/),p=m[1];(groups[p]??=[]).push({symbol:q.symbol,kind:q.kind,line:+m[2],covering:q.covering_entries});}
console.log(JSON.stringify({area,total:qa.length,files:groups,entries:entries.map(({id,title})=>({id,title}))}));
}else if(mode==='entries'){for(const e of entries)if(!args.length||args.includes(e.id))console.log(JSON.stringify(e));}
else if(mode==='file'){const p=args[0],s=source(p);const rows=qa.filter(q=>q.ts.startsWith('ice:'+p+':'));const a=args[1]?+args[1]:1,b=args[2]?+args[2]:s.lines.length;
console.log(JSON.stringify({path:p,sha256:s.sha256,total:s.lines.length,symbols:rows}));
for(let i=a-1;i<Math.min(b,s.lines.length);i++)console.log((i+1)+' | '+s.lines[i]);
}else if(mode==='declarations'){const p=args[0],s=source(p);const qs=qa.filter(q=>q.ts.startsWith('ice:'+p+':'));console.log(JSON.stringify({path:p,sha256:s.sha256,total:s.lines.length}));for(const q of qs){const a=+q.ts.split(':').at(-1);let b=Math.min(a+18,s.lines.length);for(let i=a;i<b;i++)if(/^export /.test(s.lines[i])){b=i;break;}console.log(JSON.stringify(q));for(let i=Math.max(0,a-2);i<b;i++)console.log((i+1)+' | '+s.lines[i]);}}
else if(mode==='batch'){
const files=[...new Set(qa.map(q=>q.ts.match(/^ice:(.*):\d+$/)[1]))],start=Number(args[0]??0),count=Number(args[1]??5),evidence=[];
for(const p of files.slice(start,start+count)){const s=source(p),qs=qa.filter(q=>q.ts.startsWith('ice:'+p+':'));let rs=qs.map(q=>{const a=+q.ts.split(':').at(-1);return[Math.max(1,a-1),Math.min(s.lines.length,a+(q.covering_entries.length?18:48))];}).sort((a,b)=>a[0]-b[0]);const ranges=[];for(const r of rs){if(ranges.length&&r[0]<=ranges.at(-1)[1]+1)ranges.at(-1)[1]=Math.max(ranges.at(-1)[1],r[1]);else ranges.push(r);}const rec={path:p,sha256:s.sha256,emitted_ranges:ranges,symbols:qs};evidence.push(rec);console.log(JSON.stringify(rec));for(const [a,b]of ranges)for(let i=a-1;i<b;i++)console.log((i+1)+' | '+s.lines[i]);}
fs.writeFileSync(path.join(E,'lane-exports-rpc-ai-auth-read-'+area+'-'+start+'.json'),JSON.stringify({generated_at:new Date().toISOString(),purpose:'Exact source excerpts emitted for human/model review; emission is not by itself a completed disposition',files:evidence},null,2)+'\n');
}
else throw Error('Invalid read-only mode');
