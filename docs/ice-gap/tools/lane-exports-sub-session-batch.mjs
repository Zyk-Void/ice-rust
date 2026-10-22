import { auditHelperTools } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
assertAuditOpen();
import {execFileSync} from 'node:child_process';
const [area,requests]=process.argv.slice(2);
const result=requests.split(',').map(r=>JSON.parse(execFileSync(process.execPath,[auditHelperTools+'/lane-exports-sub-session-inspect.mjs','source',area,...r.split(':')],{encoding:'utf8',maxBuffer:32*1024*1024})));
console.log(JSON.stringify(result));
