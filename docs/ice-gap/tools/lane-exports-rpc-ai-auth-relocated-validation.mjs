// Validate the actual relocated helper tree using syntax parsing and reviewed read-only validators.
// No historical authoring, publication, merger, read-credit or provenance generator is executed.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { ACTIVE_AUDIT, PREFIX, PLAN_NAME, sha, safe, inventory, syntaxCheck, assertNoPythonResidue } from './lane-exports-rpc-ai-auth-migration-core.mjs';
const base=path.dirname(path.dirname(fileURLToPath(import.meta.url)));
if(base!==ACTIVE_AUDIT||fs.realpathSync(base)!==ACTIVE_AUDIT||process.argv.length!==2)throw Error('Actual relocated validation runs only in the exact active audit without arguments.');
const read=rel=>JSON.parse(fs.readFileSync(safe(base,rel),'utf8'));
const issues=[],checks=[],syntax_results=[],executions=[],actual_helper_hashes=[];
const correction_record_path='evidence/active-closeout-corrections.json';
let plan,receipt,plan_sha256,receipt_sha256,residue,correction_record_sha256,original_helper_validation_sha256;
const problem=(kind,detail)=>issues.push({kind,...detail});
try{
 const planBytes=fs.readFileSync(safe(base,'evidence/'+PLAN_NAME));plan=JSON.parse(planBytes);plan_sha256=sha(planBytes);
 const receiptRel='evidence/'+PREFIX+'-migration-result.json',receiptBytes=fs.readFileSync(safe(base,receiptRel));receipt=JSON.parse(receiptBytes);receipt_sha256=sha(receiptBytes);
 if(plan_sha256!=='73bc41a761e791c640d0267b1755d58f4c9825e06677f5277bfb7bbbc03cd9c3'||receipt_sha256!=='55f09fcf112514b8311e54475624529a8ea5cdcf25360ecd110f5e384974499e')throw Error('Original migration evidence changed.');
 if(plan.status!=='ready-for-active-copy'||receipt.applied!==true||receipt.plan_sha256!==plan_sha256||receipt.active_audit_root!==base||receipt.active_head!=='1891337558105fb8322c79617e5a03d98c1b2c3a')throw Error('Matching applied helper migration receipt required.');
 const correctionBytes=fs.readFileSync(safe(base,correction_record_path)),correction=JSON.parse(correctionBytes);correction_record_sha256=sha(correctionBytes);
 const archivePath='evidence/closeout-corrections/relocated-helper-validation.before.json',archiveBytes=fs.readFileSync(safe(base,archivePath)),archive=JSON.parse(archiveBytes);
 original_helper_validation_sha256=sha(archiveBytes);
 if(original_helper_validation_sha256!=='9160fa342d5a48437181d705a6d70b0a17fa75041cf2df1063199771232e9415'||archive.passed!==true||archive.safe_execution_complete!==true||archive.plan_sha256!==plan_sha256||archive.receipt_sha256!==receipt_sha256)throw Error('Original helper validation archive changed.');
 const baselineArchivePath='evidence/closeout-corrections/helper-baselines.json',baselineArchiveBytes=fs.readFileSync(safe(base,baselineArchivePath));
 if(sha(baselineArchiveBytes)!=='bf8a3e08417974ab86af06045ee5473187ca1b8c4805dcf568336e4447ef28e0')throw Error('Helper baseline archive changed.');
 const baselineArchive=JSON.parse(baselineArchiveBytes),archivedHelpers=new Map(baselineArchive.helpers.map(r=>[r.path,r]));
 if(correction.schema_version!==1||correction.status!=='approved'||correction.source_only!==true||correction.application_or_test_execution!==false||correction.active_audit_root!==base||correction.active_head!==receipt.active_head||correction.original_plan_sha256!==plan_sha256||correction.original_migration_receipt_sha256!==receipt_sha256||correction.original_helper_validation_sha256!==original_helper_validation_sha256)throw Error('Approved active correction record does not bind the original evidence.');
 const allowedCorrections=['tools/build-final-index.py','tools/lane-exports-rpc-ai-auth-relocated-validation.mjs'];
 if(!Array.isArray(correction.helper_changes)||JSON.stringify(correction.helper_changes.map(r=>r.path).sort())!==JSON.stringify(allowedCorrections))throw Error('Exactly the builder and relocated-validator corrections are required.');
 const frozenHashes=new Map([...plan.scripts.map(r=>[r.destination,r.after_sha256]),...plan.support_files.map(r=>[r.path,r.sha256])]),corrected=new Map();
 for(const change of correction.helper_changes){
  const saved=archivedHelpers.get(change.path),frozen=frozenHashes.get(change.path);
  if(!saved||change.before_sha256!==frozen||saved.sha256!==frozen||typeof change.baseline_base64!=='string'||change.baseline_base64!==saved.content_base64||typeof change.reason!=='string'||!change.reason.trim()||typeof change.reviewed_by!=='string'||!change.reviewed_by.trim()||!Array.isArray(change.patches)||!change.patches.length)throw Error('Invalid recorded helper correction: '+change.path);
  const baseline=Buffer.from(change.baseline_base64,'base64');
  if(baseline.toString('base64')!==change.baseline_base64||sha(baseline)!==frozen||baseline.length!==saved.bytes)throw Error('Correction baseline bytes differ: '+change.path);
  let transformed=baseline.toString('utf8');
  if(!Buffer.from(transformed,'utf8').equals(baseline))throw Error('Correction baseline is not exact UTF-8: '+change.path);
  for(const patch of change.patches){
   if(typeof patch.before!=='string'||!patch.before||typeof patch.after!=='string'||patch.before===patch.after||transformed.split(patch.before).length-1!==1)throw Error('Correction patch must match exactly once: '+change.path);
   transformed=transformed.replace(patch.before,()=>patch.after);
  }
  const after=Buffer.from(transformed,'utf8'),actualBytes=fs.readFileSync(safe(base,change.path));
  if(change.after_sha256===frozen||sha(after)!==change.after_sha256||!after.equals(actualBytes))throw Error('Reconstructed helper correction differs from current bytes: '+change.path);
  corrected.set(change.path,change.after_sha256);
 }
 const stableArtifacts=[['evidence/'+PLAN_NAME,plan_sha256],[receiptRel,receipt_sha256],[correction_record_path,correction_record_sha256],[archivePath,original_helper_validation_sha256],[baselineArchivePath,sha(baselineArchiveBytes)]];
 const byDestination=new Map(receipt.scripts.map(r=>[r.destination,r]));
 const expected=[...plan.scripts.map(r=>r.destination),...plan.support_files.map(r=>r.path)].sort();
 const actual=inventory(base);
 if(expected.length!==142||actual.loose.length||JSON.stringify(actual.tools)!==JSON.stringify(expected))throw Error('Actual relocated inventory differs from the 142-helper plan.');
 if(receipt.scripts.length!==plan.scripts.length)throw Error('Receipt script count differs from plan.');
 for(const row of plan.scripts){
  const record=byDestination.get(row.destination),raw=fs.readFileSync(safe(base,row.destination));
  if(!record||record.source!==row.source||record.before_sha256!==row.before_sha256||record.after_sha256!==row.after_sha256)throw Error('Original helper receipt differs from plan: '+row.destination);
  const expectedHash=corrected.get(row.destination)??row.after_sha256,currentHash=sha(raw);
  if(currentHash!==expectedHash)throw Error('Actual helper bytes differ: '+row.destination);
  actual_helper_hashes.push({path:row.destination,baseline_sha256:row.after_sha256,sha256:currentHash,bytes:raw.length,corrected:corrected.has(row.destination)});
  if(row.move&&fs.existsSync(safe(base,row.source,true)))throw Error('Old helper remains: '+row.source);
  const result=syntaxCheck(row.destination,raw);syntax_results.push(result);
  if(!result.passed)problem('syntax',result);
 }
 for(const row of plan.support_files){
  const raw=fs.readFileSync(safe(base,row.path));
  const expectedHash=corrected.get(row.path)??row.sha256,currentHash=sha(raw);
  if(currentHash!==expectedHash)throw Error('Actual support helper differs: '+row.path);
  actual_helper_hashes.push({path:row.path,baseline_sha256:row.sha256,sha256:currentHash,bytes:raw.length,corrected:corrected.has(row.path)});
  const result=syntaxCheck(row.path,raw);syntax_results.push(result);if(!result.passed)problem('syntax',result);
 }
 if(fs.existsSync(safe(base,'independent-review',true)))throw Error('Independent review was not moved.');
 for(const row of plan.independent_review.files)if(sha(fs.readFileSync(safe(base,row.destination)))!==row.sha256)throw Error('Independent review findings changed: '+row.destination);
 residue=assertNoPythonResidue(base);
 if(syntax_results.length!==142||actual_helper_hashes.filter(r=>r.corrected).length!==2)throw Error('Incomplete original-plus-correction helper coverage.');
 checks.push('Original migration plan, receipt and prior helper validation remain hash-identical. Only the two approved helper corrections differ; their exact patches reconstruct current bytes from archived originals.','All 142 current helper hashes, paths, static inventory and unchanged independent-review hashes match the original evidence plus the approved correction.','Every helper was parsed, without importing or executing historical mutation code.');
 function run(script,args,assess){
  const target=safe(base,'tools/'+script),result=spawnSync(process.execPath,[target,...args],{cwd:base,encoding:'utf8',timeout:20000,maxBuffer:8*1024*1024,env:{...process.env,GIT_OPTIONAL_LOCKS:'0',GIT_TERMINAL_PROMPT:'0'}});
  let parsed=null,assessment=[];
  try{parsed=JSON.parse(result.stdout??'');if(result.status!==0||result.error)assessment.push({kind:'process',status:result.status,error:result.error?.message??null});assessment.push(...assess(parsed));}
  catch(error){assessment.push({kind:'validator-output',error:error.message});}
  const record={script:'tools/'+script,args,cwd:base,helper_sha256:sha(fs.readFileSync(target)),status:result.status,passed:assessment.length===0,stdout_sha256:sha(result.stdout??''),stderr:result.stderr?.trim()??'',summary:parsed,issues:assessment};
  executions.push(record);for(const p of assessment)problem('safe-validator',{script,detail:p});
 }
 const areas=read('evidence/progress.json').areas.map(r=>r.area);
 if(areas.length!==16||new Set(areas).size!==16)throw Error('Unexpected current area inventory.');
 for(const area of areas)run('inspect-spec.mjs',[area],r=>(r.area!==area?[{kind:'area-mismatch'}]:[]).concat(r.problems??[{kind:'missing-problems'}]));
 run('inspect-final-structure.mjs',[],r=>{
  const keys=['duplicate_ids','missing_fields','invalid_values','unknown_dependencies','unknown_metadata_ids','published_missing_ids'];
  const bad=keys.flatMap(k=>Array.isArray(r[k])?r[k].map(value=>({kind:k,value})):[{kind:'missing-field',field:k}]);
  if(r.areas?.length!==areas.length||r.areas.some(a=>!a.accepted||!a.published))bad.push({kind:'incomplete-area-publication'});return bad;
 });
 run('verify-supplemental-baseline.mjs',['rust','crates/ice-cli/src/bin/icers.rs'],r=>r.rows?.length===1&&r.rows[0].matches_head===true&&r.rows[0].head==='1891337558105fb8322c79617e5a03d98c1b2c3a'?[]:[{kind:'active-binary-baseline'}]);
 residue=assertNoPythonResidue(base);
 for(const row of actual_helper_hashes)if(sha(fs.readFileSync(safe(base,row.path)))!==row.sha256)throw Error('Helper changed during validation: '+row.path);
 const finalInventory=inventory(base);
 if(finalInventory.loose.length||JSON.stringify(finalInventory.tools)!==JSON.stringify(expected))throw Error('Helper inventory changed during validation.');
 for(const [rel,expectedHash]of stableArtifacts)if(sha(fs.readFileSync(safe(base,rel)))!==expectedHash)throw Error('Bound evidence changed during validation: '+rel);
 checks.push('Current spec validators ran for every published area from the audit-directory cwd, proving the relocated data roots work.','Current structural reconciliation and visible-file local Git baseline comparison ran without application/test/build execution.','All current helper hashes and bound original/correction evidence were rechecked after the safe executions.');
}catch(error){problem('validation-aborted',{error:error.message});}
const report={schema_version:1,captured_at:new Date().toISOString(),owner:'/root/helper_migration',active_audit_root:base,source_only:true,application_or_test_execution:false,historical_mutators_executed:false,read_credit_recorders_executed:false,safe_execution_complete:issues.length===0&&executions.length===18,passed:issues.length===0&&executions.length===18,plan_sha256,receipt_sha256,correction_record_path,correction_record_sha256,original_helper_validation_sha256,actual_helper_hashes,current_helper_inventory_sha256:sha(JSON.stringify(actual_helper_hashes)),node_version:process.version,python_command:'/usr/bin/python3 -I',summary:{helpers:syntax_results.length,syntax_passed:syntax_results.filter(r=>r.passed).length,syntax_failed:syntax_results.filter(r=>!r.passed).length,safe_validator_executions:executions.length,safe_validator_failures:executions.filter(r=>!r.passed).length,issues:issues.length},syntax_results,safe_executions:executions,python_residue:residue,checks,issues,limitations:['Syntax validation does not execute or semantically certify historical writers.','Safe validators inspect current accepted documentation and visible tracked source; they add no source-read credit.','The separate comprehensive active final gate remains the publication completion gate.','Independent review remains unfinished; its exact existing findings were only moved unchanged.']};
const target=safe(base,'evidence/relocated-helper-validation.json',true);fs.writeFileSync(target,JSON.stringify(report,null,2)+'\n');
console.log(JSON.stringify({path:'evidence/relocated-helper-validation.json',sha256:sha(fs.readFileSync(target)),passed:report.passed,safe_execution_complete:report.safe_execution_complete,summary:report.summary,issues:report.issues},null,2));
if(!report.passed)process.exitCode=1;
