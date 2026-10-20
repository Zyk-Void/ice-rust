// Coordinator-reviewed canonical coverage merge; hashes are identities, never source-read credit.
import fs from 'node:fs';import path from 'node:path';import crypto from 'node:crypto';import{fileURLToPath}from'node:url';
const E=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../evidence');
const sha=x=>crypto.createHash('sha256').update(x).digest('hex');
if(JSON.parse(fs.readFileSync(path.join(E,'progress.json'),'utf8')).complete)throw Error('Published tree is protected');
const name='lane-final-areas-read-coverage-proposal.json',raw=fs.readFileSync(path.join(E,name)),q=JSON.parse(fs.readFileSync(path.join(E,'lane-final-areas-read-coverage-qa.json'),'utf8')),p=JSON.parse(raw);
if(sha(raw)!=='3cf186a6e62e536dabcb60d903ee186f3a6d6e974991c70de268b4638909417e'||q.proposal_sha256!==sha(raw)||q.status!=='PASS'||q.validation.errors.length)throw Error('Proposal QA failed');
const current=fs.readFileSync(path.join(E,'read-coverage.json')),old=JSON.parse(current),files=old.files??old;
if(Object.keys(files).some(k=>!p.files[k]))throw Error('Lost legacy coverage record');
for(const[k,r]of Object.entries(p.files)){for(const l of [...(r.excluded_redacted_lines??[]),...(r.excluded_unverified_lines??[])])if(r.ranges.some(([a,b])=>a<=l&&l<=b))throw Error('Excluded line credited '+k+':'+l);}
if(Object.keys(p.files).length!==1702||p.baseline_identity_limitations.length!==5||p.coverage_policies.length!==1)throw Error('Unexpected proposal shape');
const saved='read-coverage-before-final-reconciliation.json';if(fs.existsSync(path.join(E,saved)))throw Error('Already applied');
fs.writeFileSync(path.join(E,saved),current);
p.coordinator_acceptance={captured_at:new Date().toISOString(),owner:'/root',proposal:name,proposal_sha256:sha(raw),previous_sha256:sha(current),previous_snapshot:saved,review:'Reviewed all summary/policy/copy proof fields and five current-only historical limits. Preserve explicit redactions, sessions.rs627 conflict, eight legacy-only records, and separately declared generated-data grouping. No new independent source reads are claimed by this merge.'};
fs.writeFileSync(path.join(E,'read-coverage.json'),JSON.stringify(p,null,2)+'\n');
console.log(JSON.stringify({status:'accepted',files:Object.keys(p.files).length,read_sha256:sha(fs.readFileSync(path.join(E,'read-coverage.json'))),summary:p.summary},null,2));
