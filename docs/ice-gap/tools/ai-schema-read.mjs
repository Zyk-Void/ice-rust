import { auditHelperEvidence } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
// Read-only targeted AI schema audit metadata. No source imports, writes, application execution, tests, or network.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {fileURLToPath} from 'node:url';
const base=auditHelperEvidence;
const digest=s=>crypto.createHash('sha256').update(s).digest('hex');
const [mode,...args]=process.argv.slice(2);
if(mode==='candidates'){
 const name='candidates-ai.json',raw=fs.readFileSync(path.join(base,name),'utf8'),parsed=JSON.parse(raw);
 const all=Array.isArray(parsed)?parsed:parsed.candidates;
 if(!Array.isArray(all))throw Error('Unknown candidate shape');
 const rows=all.filter(c=>c.path==='packages/ai/src/types.ts');
 console.log(JSON.stringify({path:name,sha256:digest(raw),count:rows.length,candidates:rows}));
}else if(mode==='spec-context'){
 for(const [name,ids] of [['spec-agent.json',['AGENT-001']],['spec-ai.json',['AI-006','AI-009','AI-013']],['spec-rpc-remote.json',['RPC-003','RPC-004','RPC-034']]]){
 const raw=fs.readFileSync(path.join(base,name),'utf8'),data=JSON.parse(raw);
 console.log(JSON.stringify({path:name,sha256:digest(raw),entries:data.entries.filter(e=>ids.includes(e.id))}));
 }
}else if(mode==='own-metadata'){
 const allowed=new Set(['spec-ai-schema-supplement.json','proposal-ai-schema-supplement.json','ai-schema-source-review.json','ai-schema-qa.json','ai-schema-defect-candidates.json','checkpoint-ai-schema.md','ai-schema-dependency-amendment.json']);
 for(const name of args){if(!allowed.has(name))throw Error('Unowned artifact');const raw=fs.readFileSync(path.join(base,name),'utf8');console.log(JSON.stringify({path:name,bytes:Buffer.byteLength(raw),sha256:digest(raw),lines:raw.split('\n').length}));}
}else throw Error('Invalid mode');
