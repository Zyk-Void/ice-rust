// Read-only first-pass coverage inspection of authored audit JSON; never mutates source or shared evidence.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {fileURLToPath} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../evidence');
const [mode,...args]=process.argv.slice(2);
const allowed=/^(?:(?:spec-|proposal-)?(?:ai|agent|tools|session|settings|extensions|rpc-remote|tui|subagents|safe-verify|observability|cognee|blackhole|providers-auth|cli-packaging|misc)(?:-terminal|-supplement)?|rust-defects)\.json$/;
const read=name=>{if(!allowed.test(name))throw Error('Not an audit coverage input');const raw=fs.readFileSync(path.join(root,name),'utf8');return{name,sha256:crypto.createHash('sha256').update(raw).digest('hex'),bytes:Buffer.byteLength(raw),data:JSON.parse(raw)};};
if(mode==='index'||mode==='claims'){
 for(const name of args){const r=read(name);const d=r.data;console.log(JSON.stringify({name:r.name,sha256:r.sha256,bytes:r.bytes,area:d.area,complete:d.complete,draft_complete:d.draft_complete,keys:Object.keys(d),entries:mode==='index'?(d.entries??[]).map(({id,title,status,dependencies})=>({id,title,status,dependencies})):undefined,claims:d.claims??[],arrays:Object.fromEntries(Object.entries(d).filter(([,v])=>Array.isArray(v)).map(([k,v])=>[k,v.length]))}));}
}else if(mode==='entries'){
 const [name,...ids]=args;const r=read(name);console.log(JSON.stringify({name:r.name,sha256:r.sha256,bytes:r.bytes,entries:(r.data.entries??[]).filter(e=>ids.includes(e.id))}));
}else if(mode==='surfaces'){
 const [name,...needles]=args;const r=read(name);console.log(JSON.stringify({name:r.name,sha256:r.sha256,bytes:r.bytes,surfaces:(r.data.surfaces??[]).filter(s=>needles.some(n=>s.name.includes(n)||s.entry_id===n)),exclusions:(r.data.candidate_exclusions??[]).filter(s=>needles.some(n=>JSON.stringify(s).includes(n)))}));
}else throw Error('Unknown inspection mode');
