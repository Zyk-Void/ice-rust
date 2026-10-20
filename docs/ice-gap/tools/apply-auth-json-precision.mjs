import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
assertAuditOpen();
// Apply the coordinator-reviewed AUTH JSON-helper precision addition.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {fileURLToPath} from 'node:url';
const base=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..'),E=path.join(base,'evidence');
const p=JSON.parse(fs.readFileSync(path.join(E,'auth-json-helper-precision.json'),'utf8'));
const specName='spec-'+p.area+'.json',spec=JSON.parse(fs.readFileSync(path.join(E,specName),'utf8'));
const e=spec.entries.find(e=>e.id===p.entry_id);if(!e||e.title!==p.expected_title)throw Error('Reviewed target changed');
if(e.mechanism.includes('The shared parseJsonWithComments helper first calls strict JSON.parse.'))throw Error('Precision patch already applied');
const before=JSON.parse(JSON.stringify(e));
for(const[k,v]of Object.entries(p.append_fields))e[k]+=v;
const writes=new Map([[specName,spec]]);
const merge=rs=>{const out=[];for(const r of [...rs].sort((a,b)=>a[0]-b[0])){const x=out.at(-1);if(x&&r[0]<=x[1]+1)x[1]=Math.max(x[1],r[1]);else out.push([...r]);}return out;};
for(const name of [p.area+'.json','proposal-'+p.area+'.json']){const f=path.join(E,name);if(!fs.existsSync(f))continue;const m=JSON.parse(fs.readFileSync(f,'utf8')),me=m.entries.find(x=>x.id===e.id);
if(!me)throw Error('Manifest target missing '+name);
for(const k of Object.keys(me))if(k in e)me[k]=e[k];
for(const r of p.read_evidence){const old=m.read_evidence.find(x=>x.path===r.path);if(old){if(old.sha256&&old.sha256!==r.sha256)throw Error('Read hash changed '+r.path);old.ranges=merge([...old.ranges,...r.ranges]);old.notes=(old.notes??'')+' Additional independent JSON-parser precision check: evidence/auth-json-helper-precision.json.';}else m.read_evidence.push(r);}
writes.set(name,m);}
const history={source_only:true,entry_id:e.id,before,after:e,evidence:'evidence/auth-json-helper-precision.json',changed_files:[...writes.keys()]};
fs.writeFileSync(path.join(E,'auth-json-helper-precision-applied.json'),JSON.stringify(history,null,2)+'\n');
for(const[name,data]of writes)fs.writeFileSync(path.join(E,name),JSON.stringify(data,null,2)+'\n');
console.log(JSON.stringify({entry:e.id,files:[...writes.keys()].map(name=>({name,sha256:crypto.createHash('sha256').update(fs.readFileSync(path.join(E,name))).digest('hex')}))},null,2));
