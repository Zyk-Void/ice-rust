import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
assertAuditOpen();
// Repair literal JSON fields after the connector's replacement-string expansion.
import fs from 'node:fs';import path from 'node:path';import {fileURLToPath} from 'node:url';
const base=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const file=path.join(base,'evidence/spec-agent.json');
const original=fs.readFileSync(file,'utf8');
const p=JSON.parse(fs.readFileSync(path.join(base,'evidence/agent57-literal-correction.json'),'utf8'));
const lines=original.split('\n');const id=lines.findIndex(l=>l.includes('"id": "'+p.entry+'"'));if(id<0)throw Error('Entry not found');
let end=lines.findIndex((l,i)=>i>id&&l.includes('"id": "'));if(end<0)end=lines.length;
for(const key of ['mechanism','interfaces']){
const hits=[];for(let i=id;i<end;i++)if(lines[i].trimStart().startsWith('"'+key+'":'))hits.push(i);
if(hits.length!==1)throw Error('Expected unique '+key);
lines[hits[0]]='      "'+key+'": '+JSON.stringify(p[key])+',';
}
const corrected=lines.join('\n');const parsed=JSON.parse(corrected);
if(parsed.entries.length!==65)throw Error('Entry count changed');
fs.writeFileSync(path.join(base,'evidence/spec-agent-tool-edit-failure-2026-10-19.json.txt'),original);
fs.writeFileSync(file,corrected);
console.log(JSON.stringify({corrected:p.entry,entries:parsed.entries.length,fields:['mechanism','interfaces'],cause:'Connector edit used replacement-string metasequences; literal line assignment used and JSON parsed before write.'}));
