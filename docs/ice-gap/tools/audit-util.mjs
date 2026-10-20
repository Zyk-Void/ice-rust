import { auditHelperBase, auditHelperNative } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
if(["record-reads"].includes(process.argv[2]))assertAuditOpen();
// Audit bookkeeping and read-only inspections. All writes are confined to this directory.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
const base=auditHelperBase;
const roots={ice:'/home/mewtwo/Zks/ice',rust:auditHelperNative};
const [command,...args]=process.argv.slice(2);
function load(rel,fallback){try{return JSON.parse(fs.readFileSync(path.join(base,rel),'utf8'));}catch(e){if(e.code==='ENOENT')return fallback;throw e;}}
function write(rel,content){const p=path.resolve(base,rel);if(!p.startsWith(base+path.sep))throw Error('Output outside allowed root');fs.mkdirSync(path.dirname(p),{recursive:true});fs.writeFileSync(p,content);}
function resolve(ref){const m=/^(ice|rust):(.+)$/.exec(ref);if(!m)throw Error('Expected ice: or rust: path');const p=path.resolve(roots[m[1]],m[2]);if(!p.startsWith(roots[m[1]]+path.sep))throw Error('Read outside repos');return p;}
function merge(ranges){const sorted=ranges.sort((a,b)=>a[0]-b[0]);const out=[];for(const r of sorted){const p=out.at(-1);if(p&&r[0]<=p[1]+1)p[1]=Math.max(p[1],r[1]);else out.push([...r]);}return out;}
if(command==='record-reads'){
 const data=load('evidence/read-coverage.json',{});
 for(const spec of args){const m=/^(ice|rust):(.+):(\d+)-(\d+)$/.exec(spec);if(!m)throw Error('Bad read range '+spec);const ref=m[1]+':'+m[2];const buf=fs.readFileSync(resolve(ref));const start=Number(m[3]),end=Number(m[4]);const old=data[ref]??{ranges:[]};data[ref]={...old,ranges:merge([...old.ranges,[start,end]]),sha256:crypto.createHash('sha256').update(buf).digest('hex'),recorded_at:new Date().toISOString()};}
 write('evidence/read-coverage.json',JSON.stringify(data,null,2)+'\n');console.log('Recorded actual read ranges for '+args.length+' files.');
}else if(command==='area-files'){
 const ledger=load('evidence/tracked-files.json',[]);for(const r of ledger.filter(r=>r.area===args[0]))console.log(r.lines+'\t'+r.path);
}else if(command==='rust-files'){
 const files=load('evidence/rust-tracked-files.json',[]).filter(p=>!args[0]||p.includes(args[0]));for(const p of files){const full=resolve('rust:'+p);try{if(!fs.statSync(full).isFile())continue;const buf=fs.readFileSync(full);if(buf.includes(0))continue;const text=buf.toString();console.log((text.split('\n').length-(text.endsWith('\n')?1:0))+'\t'+p);}catch(e){console.log('unreadable\t'+p+'\t'+e.code);}}
}else if(command==='contracts'){
 const cs=load('evidence/type-contracts.json',[]).filter(c=>c.area===args[0]&&(!args[1]||c.name.includes(args[1])));for(const c of cs)console.log(c.path+':'+c.line+'-'+c.end_line+'\n'+c.text+'\n');
}else if(command==='progress'){
 const ledger=load('evidence/tracked-files.json',[]);const reads=load('evidence/read-coverage.json',{});const full=ledger.filter(r=>reads['ice:'+r.path]?.ranges.some(x=>x[0]===1&&x[1]>=r.lines));console.log(JSON.stringify({ledger_rows:ledger.length,fully_read_rows:full.length,remaining_read_rows:ledger.length-full.length,by_area:Object.fromEntries([...new Set(ledger.map(r=>r.area))].map(a=>[a,{total:ledger.filter(r=>r.area===a).length,fully_read:full.filter(r=>r.area===a).length}]))},null,2));
}else throw Error('Unknown audit-util command: '+command);
