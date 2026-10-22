import { auditHelperNative } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
// Read-only range reader for files exceeding the connector's whole-file cap.
// Use only repository paths; this does not record a file as read automatically.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
const roots={ice:'/home/mewtwo/Zks/ice',rust:auditHelperNative};
for(const arg of process.argv.slice(2)){
 const m=/^(ice|rust):(.+):(\d+)-(\d+)$/.exec(arg);
 if(!m)throw Error('Expected repository:path:start-end');
 const file=path.resolve(roots[m[1]],m[2]);
 if(!file.startsWith(roots[m[1]]+path.sep))throw Error('Outside authorized repository');
 const buf=fs.readFileSync(file);if(buf.includes(0))throw Error('Binary file');
 const lines=buf.toString('utf8').split('\n');const start=Number(m[3]),end=Math.min(Number(m[4]),lines.length);
 if(start<1||start>end)throw Error('Bad range');
 console.log('\n### '+arg+' total_lines='+lines.length+' sha256='+crypto.createHash('sha256').update(buf).digest('hex'));
 for(let i=start-1;i<end;i++)console.log(String(i+1).padStart(6)+' | '+lines[i]);
}
