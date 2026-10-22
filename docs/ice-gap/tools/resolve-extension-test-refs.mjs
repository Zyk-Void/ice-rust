import { auditHelperBase, auditHelperTools } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
assertAuditOpen();
// Pin references to test AST spans at directly read line numbers; never run a test.
import fs from 'node:fs';
const base=auditHelperBase;
const helper=auditHelperTools+'/author-extensions-common.mjs';let h=fs.readFileSync(helper,'utf8');
const a='n.arguments[0].text.includes(needle)';
const b='(typeof needle === "number" ? sf.getLineAndCharacterOfPosition(n.getStart(sf)).line+1 === needle : n.arguments[0].text.includes(needle))';
if(h.includes(a))h=h.replace(a,b);fs.writeFileSync(helper,h);
const file=auditHelperTools+'/author-extensions-dispatch.mjs';let s=fs.readFileSync(file,'utf8');
const refs={
 'queued provider registrations':868,'all queued definitions':891,'delegate registration':915,
 'getScopedModels':107,'bound compaction state live':123,'project trust state':547,'live bound abort signal':519,
 'fork options':940,'passes patched tool_result':776,'preserves tool_result content':823,
 'in-place mutations':981,'still runs later':1001,'getSystemPrompt in sync':732,'stops on the first explicit':137
};
for(const [name,line]of Object.entries(refs))s=s.replaceAll("test('extensions-runner.test.ts','"+name+"')","test('extensions-runner.test.ts',"+line+")");
fs.writeFileSync(file,s);console.log('Pinned existing test references by AST start line.');
