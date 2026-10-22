import { auditHelperEvidence } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
// Extensions-owned inspection helper. Does not load application modules or modify source.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
const base=auditHelperEvidence;
const roots={ice:'/home/mewtwo/Zks/ice',rust:'/home/mewtwo/Zks/ice-rust'};
const manifest=JSON.parse(fs.readFileSync(path.join(base,'resume-source-identities-2026-10-19T222055Z.json'),'utf8'));
const spec=()=>JSON.parse(fs.readFileSync(path.join(base,'spec-extensions.json'),'utf8'));
const [mode,...args]=process.argv.slice(2);
if(mode==='overview') console.log(JSON.stringify(spec().entries.map(({id,title,status,source})=>({id,title,status,source})),null,2));
else if(mode==='entries') console.log(JSON.stringify(spec().entries.slice(Number(args[0]),Number(args[1])),null,2));
else if(mode==='candidates') {const c=JSON.parse(fs.readFileSync(path.join(base,'candidates-extensions.json'),'utf8'));console.log(JSON.stringify(c.slice(Number(args[0]||0),Number(args[1]||c.length)).map(({candidate_id,category,name,path,line,end_line,kind,detail})=>({candidate_id,category,name,path,line,end_line,kind,detail})),null,2));}
else if(mode==='read') {
 for(const arg of args){const m=/^(ice|rust):(.+):(\d+)-(\d+)$/.exec(arg);if(!m)throw Error('Bad ref');
 const file=path.resolve(roots[m[1]],m[2]);if(!file.startsWith(roots[m[1]]+path.sep))throw Error('Outside root');
 const b=fs.readFileSync(file),hash=crypto.createHash('sha256').update(b).digest('hex'),baseline=manifest.hashes[m[1]][m[2]],lines=b.toString('utf8').split('\n');
 if(baseline&&baseline!==hash)throw Error('Source drift '+arg);
 const start=Number(m[3]),end=Math.min(Number(m[4]),lines.length);
 console.log(JSON.stringify({ref:arg,total_lines:lines.length,sha256:hash,baseline:baseline??null,match:baseline?baseline===hash:null}));
 for(let i=start-1;i<end;i++) console.log(String(i+1).padStart(5)+' | '+lines[i]);
 }
} else throw Error('unknown mode');
