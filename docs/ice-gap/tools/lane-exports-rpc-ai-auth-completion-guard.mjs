// Historical mutation guard. Importing this module alone never blocks read-only inspection.
import fs from 'node:fs';
import path from 'node:path';
import { auditHelperEvidence } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
export function assertAuditOpen(){
 const progress=JSON.parse(fs.readFileSync(path.join(auditHelperEvidence,'progress.json'),'utf8'));
 if(typeof progress.complete!=='boolean')throw Error('Audit completion state is missing or invalid; historical mutation refused.');
 if(progress.complete)throw Error('Historical audit mutation refused because progress.complete is true. Use the reviewed current maintenance process.');
}
