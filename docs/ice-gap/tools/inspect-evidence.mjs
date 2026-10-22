import { auditHelperBase } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
// Read-only audit inventory projection; avoids huge JSON tool responses.
import fs from 'node:fs';
import path from 'node:path';
const base=auditHelperBase;
const [command,area]=process.argv.slice(2);
const load=p=>JSON.parse(fs.readFileSync(path.join(base,p),'utf8'));
if(command==='candidates'){
 for(const c of load('evidence/surface-candidates.json').filter(c=>!area||c.area===area))console.log([c.candidate_id,c.category,c.name,c.path+':'+c.line,c.kind,c.detail??''].join('\t').replaceAll('\n',' '));
}else if(command==='symbols'){
 for(const c of load('evidence/ts-symbols.json').filter(c=>!area||c.area===area))console.log(JSON.stringify(c));
}else if(command==='files'){
 for(const c of load('evidence/tracked-files.json').filter(c=>!area||c.area===area))console.log([c.lines,c.path,c.generated??false].join('\t'));
}else throw Error('Unknown command');
