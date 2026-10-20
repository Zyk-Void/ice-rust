// Apply the reviewed helper-only migration in the copied active audit; never in the original tree.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import { ACTIVE_AUDIT, PREFIX, PLAN_NAME, QA_NAME, sha, safe, inventory, readPreparation } from './lane-exports-rpc-ai-auth-migration-core.mjs';
const base=path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const mode=process.argv[2]??'--preview';
if(!['--preview','--apply'].includes(mode)||process.argv.length>3)throw Error('Use --preview or --apply; no alternate roots are accepted.');
if(base!==ACTIVE_AUDIT||fs.realpathSync(base)!==ACTIVE_AUDIT)throw Error('This migration runs only in the exact copied active docs/ice-gap tree.');
const read=rel=>JSON.parse(fs.readFileSync(safe(base,rel),'utf8'));
const plan=read('evidence/'+PLAN_NAME);
const gate=read('evidence/original-checkout-validation.json');
if(gate.phase!=='original'||gate.passed!==true||gate.source_only!==true||gate.application_or_test_execution!==false||!Array.isArray(gate.issues)||gate.issues.length)throw Error('A preserved passing original-checkout gate is required.');
const originalHeads={ice:'3ab05193147f5abb8eb8ee6c0178f6dbce8918b0',rust:'ff9270f26e263ea9ce54349786bf2d93c4517476'};
for(const [side,head]of Object.entries(originalHeads))if(gate.identities?.[side]?.head!==head)throw Error('Wrong original source identity: '+side);
if(gate.identities.ice.root!=='/home/mewtwo/Zks/ice'||gate.identities.rust.root!=='/home/mewtwo/Zks/ice-rust')throw Error('Wrong original source roots.');
const nativeRoot=path.resolve(base,'../..');
const activeHead=execFileSync('git',['--no-optional-locks','-C',nativeRoot,'rev-parse','HEAD'],{encoding:'utf8',env:{...process.env,GIT_TERMINAL_PROMPT:'0'}}).trim();
if(activeHead!=='1891337558105fb8322c79617e5a03d98c1b2c3a')throw Error('Active native HEAD differs from the reviewed source map.');
const receiptRel='evidence/'+PREFIX+'-migration-result.json';
const backupRel='evidence/'+PREFIX+'-migration-originals.json';
const stageRel='tools/'+PREFIX+'-migration-stage';
const receiptPath=safe(base,receiptRel,true);
function assertFinalInventory(){
 const files=inventory(base),expected=[...plan.scripts.map(s=>s.destination),...plan.support_files.map(s=>s.path)].sort();
 if(files.loose.length||JSON.stringify(files.tools)!==JSON.stringify(expected))throw Error('Post-migration script inventory differs from the reviewed complete map.');
 if(fs.existsSync(safe(base,plan.independent_review.source,true)))throw Error('Old independent-review directory reappeared.');
 return files;
}
if(fs.existsSync(receiptPath)){
 const receipt=read(receiptRel);
 if(receipt.applied!==true||receipt.plan_sha256!==sha(fs.readFileSync(safe(base,'evidence/'+PLAN_NAME))))throw Error('Existing migration receipt does not certify this plan.');
 if(receipt.scripts.length!==plan.scripts.length||JSON.stringify(receipt.scripts.map(s=>[s.source,s.destination]).sort())!==JSON.stringify(plan.scripts.map(s=>[s.source,s.destination]).sort()))throw Error('Receipt script map differs from the reviewed plan.');
 for(const row of receipt.scripts){
  if(sha(fs.readFileSync(safe(base,row.destination)))!==row.after_sha256)throw Error('Migrated helper changed: '+row.destination);
  if(row.source!==row.destination&&fs.existsSync(safe(base,row.source,true)))throw Error('Old helper path reappeared: '+row.source);
 }
 for(const row of plan.independent_review.files)if(sha(fs.readFileSync(safe(base,row.destination)))!==row.sha256)throw Error('Moved independent review changed: '+row.destination);
 assertFinalInventory();
 for(const row of plan.support_files)if(sha(fs.readFileSync(safe(base,row.path)))!==row.sha256)throw Error('Prepared support helper changed: '+row.path);
 console.log(JSON.stringify({applied:true,already_applied:true,receipt:receiptRel,scripts:receipt.scripts.length,loose_scripts:0},null,2));
}else{
 const progress=read('evidence/progress.json');
 if(progress.complete!==false)throw Error('Initial helper migration must occur before the active audit is marked complete.');
 const prepared=readPreparation(base),qa=read('evidence/'+QA_NAME);
 if(qa.passed!==true||qa.syntax_only!==true||qa.plan_sha256!==prepared.plan_sha256||qa.application_or_test_execution!==false)throw Error('Matching prepared syntax-only QA is required.');
 const checked=new Map(qa.scripts.map(r=>[r.destination,r]));
 for(const item of prepared.scripts){
  const q=checked.get(item.row.destination);
  if(!q||q.before_sha256!==item.row.before_sha256||q.after_sha256!==item.after_sha256)throw Error('Prepared syntax result is stale: '+item.row.source);
 }
 const expectedSyntax=[...prepared.scripts.map(s=>s.row.destination),...prepared.support.map(s=>s.path)].sort();
 if(checked.size!==prepared.scripts.length||qa.syntax_results.some(r=>r.passed!==true)||JSON.stringify(qa.syntax_results.map(r=>r.path).sort())!==JSON.stringify(expectedSyntax))throw Error('Incomplete syntax-only QA.');
 if(JSON.stringify(qa.support_files)!==JSON.stringify(prepared.support.map(s=>({path:s.path,sha256:sha(s.raw)}))))throw Error('Prepared support syntax inputs are stale.');
 const changed=prepared.scripts.filter(s=>s.row.move||s.changed);
 const preview={mode,active_audit_root:base,active_head:activeHead,original_gate_sha256:sha(fs.readFileSync(safe(base,'evidence/original-checkout-validation.json'))),plan_sha256:prepared.plan_sha256,scripts:prepared.scripts.length,moves:prepared.scripts.filter(s=>s.row.move).length,modified:prepared.scripts.filter(s=>s.changed).length,independent_review:prepared.plan.independent_review,backup:backupRel,receipt:receiptRel,actions:changed.map(s=>({source:s.row.source,destination:s.row.destination,before_sha256:s.row.before_sha256,after_sha256:s.after_sha256,role:s.row.role,guard:s.row.guard.kind}))};
 if(mode==='--preview'){
  console.log(JSON.stringify({...preview,applied:false},null,2));
 }else{
  const backupPath=safe(base,backupRel,true),stagePath=safe(base,stageRel,true);
  if(fs.existsSync(backupPath)||fs.existsSync(stagePath))throw Error('A previous backup/staging attempt exists; review it before starting another migration.');
  const originals={schema_version:1,captured_at:new Date().toISOString(),plan_sha256:prepared.plan_sha256,reason:'Exact pre-migration helper bytes for rollback; no source-review credit.',files:changed.map(s=>({source:s.row.source,destination:s.row.destination,sha256:s.row.before_sha256,mode:s.mode,content_base64:s.raw.toString('base64')})),independent_review:prepared.plan.independent_review};
  const completed=[];let reviewMoved=false,backupWritten=false,appliedSummary;
  try{
   fs.writeFileSync(backupPath,JSON.stringify(originals,null,2)+'\n',{flag:'wx',mode:0o600});backupWritten=true;
   fs.mkdirSync(stagePath);
   for(let i=0;i<changed.length;i++){
    const item=changed[i];
    if(item.changed)fs.writeFileSync(path.join(stagePath,String(i)+'.tmp'),item.after,{flag:'wx',mode:item.mode});
   }
   // Recheck every input immediately before the first rename, after staging is complete.
   const fresh=readPreparation(base);
   if(fresh.plan_sha256!==prepared.plan_sha256||sha(fs.readFileSync(safe(base,'evidence/original-checkout-validation.json')))!==preview.original_gate_sha256||read('evidence/progress.json').complete!==false)throw Error('Migration plan, original gate or progress changed during staging.');
   for(let i=0;i<changed.length;i++){
    const item=changed[i],row=item.row,source=safe(base,row.source),destination=safe(base,row.destination,true);
    if(row.move){
     if(fs.existsSync(destination))throw Error('Concurrent destination collision: '+row.destination);
     fs.renameSync(source,destination);
    }
    completed.push(item);
    if(item.changed)fs.renameSync(path.join(stagePath,String(i)+'.tmp'),destination);
    fs.chmodSync(destination,item.mode);
   }
   const review=prepared.plan.independent_review;
   if(fs.existsSync(safe(base,review.destination,true)))throw Error('Concurrent independent-review collision.');
   fs.renameSync(safe(base,review.source),safe(base,review.destination,true));reviewMoved=true;
   for(const item of prepared.scripts)if(sha(fs.readFileSync(safe(base,item.row.destination)))!==item.after_sha256)throw Error('Post-write helper hash mismatch: '+item.row.destination);
   for(const row of review.files)if(sha(fs.readFileSync(safe(base,row.destination)))!==row.sha256)throw Error('Post-move independent-review hash mismatch: '+row.destination);
   assertFinalInventory();
   fs.rmdirSync(stagePath);
   const receipt={schema_version:1,captured_at:new Date().toISOString(),applied:true,source_only:true,application_or_test_execution:false,plan_sha256:prepared.plan_sha256,prepared_qa_sha256:sha(fs.readFileSync(safe(base,'evidence/'+QA_NAME))),original_gate_sha256:preview.original_gate_sha256,active_audit_root:base,active_head:activeHead,backup:backupRel,backup_sha256:sha(fs.readFileSync(backupPath)),scripts:prepared.scripts.map(s=>({source:s.row.source,destination:s.row.destination,before_sha256:s.row.before_sha256,after_sha256:s.after_sha256,changed:s.changed,role:s.row.role,guard:s.row.guard.kind})),independent_review:review,checks:['All reviewed input hashes matched immediately before mutation.','Every output hash matches the syntax-checked prepared transformation.','Every root/evidence .mjs/.py is in tools.','Independent-review bytes are unchanged and its incomplete status is retained.','Historical helpers and application code were not executed.'],validation_reruns_performed:false};
   fs.writeFileSync(receiptPath,JSON.stringify(receipt,null,2)+'\n',{flag:'wx'});
   appliedSummary={applied:true,moves:preview.moves,modified:preview.modified,receipt:receiptRel,backup:backupRel,independent_review:review.destination};
  }catch(error){
   const rollbackErrors=[];
   if(reviewMoved)try{fs.renameSync(safe(base,'evidence/independent-review'),safe(base,'independent-review',true));}catch(e){rollbackErrors.push(e.message);}
   for(const item of [...completed].reverse())try{
    const row=item.row,source=safe(base,row.source,true),destination=safe(base,row.destination,true);
    fs.writeFileSync(source,item.raw,{mode:item.mode});fs.chmodSync(source,item.mode);
    if(row.move&&fs.existsSync(destination))fs.unlinkSync(destination);
   }catch(e){rollbackErrors.push(e.message);}
   if(fs.existsSync(stagePath))try{for(const name of fs.readdirSync(stagePath))fs.unlinkSync(path.join(stagePath,name));fs.rmdirSync(stagePath);}catch(e){rollbackErrors.push(e.message);}
   const failure={applied:false,captured_at:new Date().toISOString(),error:error.message,rollback_errors:rollbackErrors,backup:backupWritten?backupRel:null,plan_sha256:prepared.plan_sha256};
   try{fs.writeFileSync(safe(base,'evidence/'+PREFIX+'-migration-failure.json',true),JSON.stringify(failure,null,2)+'\n',{flag:'wx'});}catch(e){rollbackErrors.push(e.message);}
   throw Error(JSON.stringify(failure));
  }
  console.log(JSON.stringify(appliedSummary,null,2));
 }
}
