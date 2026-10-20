import { auditHelperBase } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
assertAuditOpen();
// Correct the authored draft against direct source/test reads. Output stays under docs/ice-gap.
import fs from 'node:fs';
const base=auditHelperBase;
const file=base+'/author-extensions-dispatch.mjs';
let text=fs.readFileSync(file,'utf8');
const changes=[
 ["'invalid queued provider'","'queued provider registrations'"],
 ["'removes every queued queued provider'","'all queued definitions'"],
 ["'removes every queued provider'","'all queued definitions'"],
 ["'post-bind provider'","'delegate registration'"],
 ["'current scoped models'","'getScopedModels'"],
 ["'abort signal'","'live bound abort signal'"],
 ["'forwards fork positioning'","'fork options'"],
 ["'chains tool_result'","'passes patched tool_result'"],
 ["'preserves omitted fields'","'preserves tool_result content'"],
 ["'throws, preserving earlier'","'still runs later'"],
 ["'undecided handlers'","'stops on the first explicit'"],
];
for(const [a,b]of changes)text=text.replaceAll(a,b);
fs.writeFileSync(file,text);console.log('Updated exact existing test-name references.');
