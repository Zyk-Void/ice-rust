import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
assertAuditOpen();
// Remove only two accidental duplicate audit draft aliases after canonical files were saved.
import fs from 'node:fs';
import path from 'node:path';
const base=path.resolve(import.meta.dirname,'../evidence');
for (const [alias,canonical] of [['session-spec.json','spec-session.json'],['session-proposal.json','proposal-session.json']]) {
 if (!fs.existsSync(path.join(base,canonical))) throw Error('Missing canonical '+canonical);
 if (fs.existsSync(path.join(base,alias))) fs.unlinkSync(path.join(base,alias));
}
console.log('Removed two accidental audit-only draft aliases.');
