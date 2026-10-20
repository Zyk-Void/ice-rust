import { auditHelperBase } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
assertAuditOpen();
// Render authored, source-verified specifications. This is documentation tooling, not a source test.
import fs from 'node:fs';
import path from 'node:path';
const base=auditHelperBase;
const area=process.argv[2];
if(!/^[a-z-]+$/.test(area??''))throw Error('Area required');
const data=JSON.parse(fs.readFileSync(path.join(base,'evidence',`spec-${area}.json`),'utf8'));
const heading=['Title and status','ice source','What it does','How it works','Interfaces','Dependencies','ice-rust today','Port notes','Size and priority'];
const seen=new Set();
let text=`# ${data.title}\n\n${data.introduction??''}\n\n`;
for(const e of data.entries){
 if(seen.has(e.id))throw Error('Duplicate ID '+e.id);seen.add(e.id);
 if(!['missing','partial','different-by-design'].includes(e.status))throw Error('Invalid status '+e.id);
 const fields=[`${e.title}. **Status: ${e.status}.**`,e.source,e.what,e.mechanism,e.interfaces,e.dependencies?.length?e.dependencies.map(d=>'`'+d+'`').join(', ')+(e.dependency_text?'\n\n'+e.dependency_text:''):(e.dependency_text||'none'),e.rust,e.port,`Estimated Rust implementation: **${e.size_min}-${e.size_max} lines** (estimate, not measured source). **${e.priority}** - ${e.priority_reason}`];
 for(let i=0;i<fields.length;i++)if(typeof fields[i]!=='string'||!fields[i].trim())throw Error('Missing field '+(i+1)+' '+e.id);
 text+=`## ${e.id} - ${e.title}\n\n`+fields.map((f,i)=>`### ${i+1}. ${heading[i]}\n\n${f}\n\n`).join('');
}
if(data.appendix)text+='## Exact supporting contracts\n\n'+data.appendix+'\n';
fs.writeFileSync(path.join(base,area+'.md'),text);
console.log(JSON.stringify({area,entries:data.entries.length,bytes:Buffer.byteLength(text),path:path.join(base,area+'.md')},null,2));
