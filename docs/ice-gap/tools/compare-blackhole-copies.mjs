import { auditHelperEvidence } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
assertAuditOpen();
// Compare only local tracked source copies; no network or source writes.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {execFileSync} from 'node:child_process';
const root='/home/mewtwo/Zks/ice';
const output=auditHelperEvidence+'/blackhole-copies.json';
const tracked=new Set(execFileSync('git',['-C',root,'ls-files','-z'],{encoding:'utf8'}).split('\0'));
const prefix='packages/coding-agent/examples/extensions/ice-blackhole/';
const rows=[];
for(const p of [...tracked].filter(p=>p.startsWith(prefix)&&!p.includes('.test.')&&!p.includes('/node_modules/'))){
 const other='user-extensions/ice-blackhole/'+p.slice(prefix.length);
 if(!fs.existsSync(path.join(root,other))){rows.push({source:p,copy:other,status:'absent'});continue;}
 const a=fs.readFileSync(path.join(root,p)),b=fs.readFileSync(path.join(root,other));
 rows.push({source:p,copy:other,tracked:tracked.has(other),same_bytes:a.equals(b),source_sha256:crypto.createHash('sha256').update(a).digest('hex'),copy_sha256:crypto.createHash('sha256').update(b).digest('hex'),copy_lines:b.toString('utf8').split('\n').length});
}
fs.writeFileSync(output,JSON.stringify(rows,null,2)+'\n');
console.log(JSON.stringify(rows,null,2));
