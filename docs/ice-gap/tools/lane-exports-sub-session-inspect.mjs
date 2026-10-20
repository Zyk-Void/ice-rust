import { auditHelperEvidence } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
if(["source"].includes(process.argv[2]))assertAuditOpen();
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
const E=auditHelperEvidence;
const T='/home/mewtwo/Zks/ice';
const q=JSON.parse(fs.readFileSync(path.join(E,'qa-unnamed-exports-2026-10-20.json'),'utf8'));
const [mode,area,arg,arg2,arg3]=process.argv.slice(2);
if(!['session','subagents'].includes(area))throw Error('lane area');
const rows=q.symbols[area];
const spec=JSON.parse(fs.readFileSync(path.join(E,'spec-'+area+'.json'),'utf8'));
if(mode==='index'){
const by={};for(const r of rows){const f=r.ts.slice(4).replace(/:\d+$/,'');(by[f]??=[]).push(r);}
console.log(JSON.stringify({area,total:rows.length,files:Object.entries(by).map(([file,symbols],index)=>({index,file,symbols:symbols.map(r=>({name:r.symbol,kind:r.kind,line:Number(r.ts.match(/:(\d+)$/)[1]),cover:r.covering_entries}))})),entries:spec.entries.map(e=>({id:e.id,title:e.title,source:e.source}))},null,2));
}else if(mode==='entry') { console.log(JSON.stringify(spec.entries.filter(e=>arg.split(',').includes(e.id)),null,2));
}else if(mode==='source'){
const files=[...new Set(rows.map(r=>r.ts.slice(4).replace(/:\d+$/,'')))]; const file=/^\d+$/.test(arg)?files[Number(arg)]:arg;if(!files.includes(file))throw Error('not assigned source');
const raw=fs.readFileSync(path.join(T,file));const ls=raw.toString('utf8').split('\n');const a=Number(arg2??1),b=Math.min(Number(arg3??ls.length),ls.length);
const lines=ls.slice(a-1,b).map((x,i)=>`${a+i}|${x}`).join('\n');
console.log(JSON.stringify({area,file,start:a,end:b,total:ls.length,sha256:crypto.createHash('sha256').update(raw).digest('hex'),symbols:rows.filter(r=>r.ts.startsWith('ice:'+file+':')).map(r=>({name:r.symbol,kind:r.kind,line:Number(r.ts.match(/:(\d+)$/)[1]),cover:r.covering_entries})),source:lines},null,2));
const f=path.join(E,'lane-exports-sub-session-source-review.json');const review=fs.existsSync(f)?JSON.parse(fs.readFileSync(f,'utf8')):{owner:'/root/exports_sub_session',method:'Direct source ranges returned by lane inspector; hash alone is not reading credit',reads:[]};review.reads.push({area,path:'ice:'+file,ranges:[[a,b]],sha256:crypto.createHash('sha256').update(raw).digest('hex'),captured_at:new Date().toISOString()});fs.writeFileSync(f,JSON.stringify(review,null,2)+'\n');
}else throw Error('mode');
