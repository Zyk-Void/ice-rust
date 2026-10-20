import { auditHelperEvidence } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import fs from 'node:fs';
import crypto from 'node:crypto';
const base=auditHelperEvidence;
const data=fs.readFileSync(base+'/candidates-tui.json');
const digest=crypto.createHash('sha256').update(data).digest('hex');
if(digest!=='30438804f9ad70c8f8b8956f3314a697932da36933e51c6d48f0ca4d09dc830f')throw Error('Candidate baseline drift');
const all=JSON.parse(data),prefix=process.argv[2]??'',groups={};
for(const c of all){if(!c.path.startsWith(prefix))continue;(groups[c.path]??=[]).push([c.candidate_id,c.category,c.name,c.line,c.end_line,c.kind,c.detail,c.rust_name_candidates]);}
console.log(JSON.stringify({original_count:all.length,sha256:digest,prefix,fields:['candidate_id','category','name','line','end_line','kind','detail','rust_name_candidates'],groups},null,2));
