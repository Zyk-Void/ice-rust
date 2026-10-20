import { auditHelperBase, auditHelperNative } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
// Structural/citation audit of authored documentation; not a source test.
import fs from 'node:fs';import path from 'node:path';
const base=auditHelperBase;const area=process.argv[2];
const spec=JSON.parse(fs.readFileSync(path.join(base,'evidence/spec-'+area+'.json'),'utf8'));
const roots={ice:'/home/mewtwo/Zks/ice',rust:auditHelperNative};const seen=new Set();const problems=[];
for(const e of spec.entries){for(const field of ['source','mechanism','interfaces','rust','port'])for(const m of (e[field]??'').matchAll(/\b(ice|rust):([^\s;`]+?):(\d+)-(\d+)/g)){const key=m[0];if(seen.has(key))continue;seen.add(key);try{const p=path.resolve(roots[m[1]],m[2]);if(!p.startsWith(roots[m[1]]+path.sep))throw Error('outside root');const n=fs.readFileSync(p,'utf8').split('\n').length;if(+m[3]<1||+m[4]<+m[3]||+m[4]>n)problems.push({id:e.id,field,ref:key,actual_lines:n});}catch(err){problems.push({id:e.id,field,ref:key,error:err.message});}}}
console.log(JSON.stringify({area,entries:spec.entries.length,unique_line_references:seen.size,problems},null,2));
