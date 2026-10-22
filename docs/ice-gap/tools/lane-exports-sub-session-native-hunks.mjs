import { auditHelperEvidence } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import fs from 'node:fs';
import crypto from 'node:crypto';
const file=auditHelperEvidence+'/active-checkout-diff-hunks-complete.json';
const raw=fs.readFileSync(file,'utf8');
const data=JSON.parse(raw);
const base=data.rows.filter(r=>['crates/ice-harness/','crates/ice-tui/'].some(prefix=>(r.active_path||r.old_path).startsWith(prefix)));
const docs=data.rows.filter(r=>{const p=r.active_path||r.old_path;return p==='crates/ice-cli/README.md'||p.startsWith('crates/ice-cli/embedded-docs/');});
const rows=[...base,...docs];
const mode=process.argv[2]||'index';
if(mode==='index')process.stdout.write(JSON.stringify({source:file,sha256:crypto.createHash('sha256').update(raw).digest('hex'),identities:data.identities,rows:rows.map((r,i)=>({index:i,path:r.active_path||r.old_path,old_path:r.old_path,active_path:r.active_path,status:r.status,old_sha256:r.old?.sha256,active_sha256:r.active?.sha256,changes:r.changes?.length,unified_hunks:(r.unified_diff?.match(/^@@ /gm)||[]).length,keys:Object.keys(r)}))}));
else if(mode==='rows'){
const indices=process.argv[3].split(',').map(Number);
process.stdout.write(JSON.stringify(indices.map(i=>({index:i,...rows[i]}))));
}else throw new Error('index|rows only');
