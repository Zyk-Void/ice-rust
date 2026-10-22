import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
assertAuditOpen();
// One-time append of independently reviewed audit finding; source code is untouched.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {fileURLToPath} from 'node:url';
const base=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const f=path.join(base,'evidence','rust-defects.json'),raw=fs.readFileSync(f);
if(crypto.createHash('sha256').update(raw).digest('hex')!=='3f8b788421954723b54a5d250cca6bded2a2543d93bec4cffce1c8316713fd01')throw Error('Catalog changed; review rather than overwrite.');
const rows=JSON.parse(raw),d=JSON.parse(fs.readFileSync(path.join(base,'evidence','native-queue-defect-candidate.json'),'utf8'));
if(rows.some(x=>x.id===d.id)||d.id!=='RUST-068')throw Error('Unexpected defect identity');
rows.push(d);fs.writeFileSync(f,JSON.stringify(rows,null,2)+'\n');
console.log(JSON.stringify({defects:rows.length,added:d.id,sha256:crypto.createHash('sha256').update(fs.readFileSync(f)).digest('hex')}));
