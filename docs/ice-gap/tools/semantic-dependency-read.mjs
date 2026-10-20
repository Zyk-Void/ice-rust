import { auditHelperEvidence } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
// Read-only semantic dependency QA over authored audit JSON. No source imports, writes, tests or network.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {fileURLToPath} from 'node:url';
const base=auditHelperEvidence;
const areas=new Set(['safe-verify','cognee','blackhole','agent','observability','ai','session','settings','tools','extensions','rpc-remote','tui','subagents','providers-auth','cli-packaging','misc']);
const [mode,...args]=process.argv.slice(2);
const read=(area,spec)=>{if(!areas.has(area))throw Error('Invalid area');const name=(spec?'spec-':'')+area+'.json';const raw=fs.readFileSync(path.join(base,name),'utf8');return {name,sha256:crypto.createHash('sha256').update(raw).digest('hex'),bytes:Buffer.byteLength(raw),data:JSON.parse(raw)};};
if(mode==='requests'||mode==='catalog'||mode==='metadata'){
 for(const area of args){const r=read(area,mode!=='requests');console.log(JSON.stringify({name:r.name,sha256:r.sha256,bytes:r.bytes,area,complete:r.data.complete,draft_complete:r.data.draft_complete,dependency_requests:mode==='requests'?(r.data.dependency_requests??[]):undefined,entries:mode!=='requests'?(r.data.entries??[]).map(e=>({id:e.id,title:e.title,status:e.status,dependencies:e.dependencies,...(mode==='metadata'?{source:e.source,entry_sha256:crypto.createHash('sha256').update(JSON.stringify(e)).digest('hex')}:{} )})):undefined}));}
}else if(mode==='entries'){
 const [area,...ids]=args;if(!ids.length)throw Error('Explicit entry IDs required');const r=read(area,true);console.log(JSON.stringify({name:r.name,sha256:r.sha256,bytes:r.bytes,entries:(r.data.entries??[]).filter(e=>ids.includes(e.id))}));
}else throw Error('Invalid mode');
