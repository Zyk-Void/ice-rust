// Reconstruct the explicit helper plan from reviewed rules; original audit preparation only.
// --freeze is used only after the coordinator has released all helper edits.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { OLD_AUDIT, ACTIVE_AUDIT, PREFIX, PLAN_NAME, sha, safe, walkFiles, inventory, transform } from './lane-exports-rpc-ai-auth-migration-core.mjs';
const base=path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const mode=process.argv[2]??'--draft';
if(!['--draft','--freeze'].includes(mode)||process.argv.length>3)throw Error('Use --draft or coordinator-released --freeze.');
if(base!==OLD_AUDIT||fs.realpathSync(base)!==OLD_AUDIT)throw Error('Plan preparation is confined to the exact original audit root.');
const read=rel=>JSON.parse(fs.readFileSync(safe(base,rel),'utf8'));
if(read('evidence/progress.json').complete!==false)throw Error('Original audit must remain incomplete while preparing relocation.');
const ruleRel='evidence/'+PREFIX+'-migration-rules.json',ruleBytes=fs.readFileSync(safe(base,ruleRel)),rules=JSON.parse(ruleBytes);
if(rules.schema_version!==1||rules.source_only!==true||rules.application_or_test_execution!==false)throw Error('Unexpected reviewed rule schema.');
const all=inventory(base),known=[...rules.scripts.map(r=>r.source),...rules.support_files].sort();
if(JSON.stringify(all.all)!==JSON.stringify(known))throw Error('Helper inventory changed outside the reviewed explicit rules.');
const scripts=rules.scripts.map(rule=>{
 const raw=fs.readFileSync(safe(base,rule.source)),before=raw.toString('utf8');
 if(!Buffer.from(before).equals(raw))throw Error('Helper is not valid UTF-8: '+rule.source);
 let after=before;
 for(const op of rule.operations){
  const count=after.split(op.old_text).length-1;
  if(count!==op.expected_occurrences)throw Error('Stale reviewed operation '+rule.source+' '+JSON.stringify(op.old_text)+' count='+count);
  after=after.split(op.old_text).join(op.new_text);
 }
 const a=before.split('\n'),b=after.split('\n');
 if(a.length!==b.length)throw Error('Operation changes line structure outside explicit prefix: '+rule.source);
 const patches=[];
 for(let i=0;i<a.length;i++)if(a[i]!==b[i]){
  if(before.split(a[i]).length-1!==1)throw Error('Changed source line is not unique: '+rule.source+':'+(i+1));
  patches.push({source_line:i+1,expected_occurrences:1,old_text:a[i],new_text:b[i],reason:rule.operations.filter(o=>a[i].includes(o.old_text)).map(o=>o.reason).join(' ')});
 }
 const symbols=[...new Set(rule.operations.flatMap(o=>o.new_text.match(/\bauditHelper(?:Base|Evidence|Native|Tools)\b/g)??[]))].sort();
 let prepend=symbols.length?"import { "+symbols.join(', ')+" } from './"+PREFIX+"-audit-paths.mjs';\n":'';
 if(rule.guard.kind==='unconditional-prefix'||rule.guard.kind==='conditional-prefix'){
  prepend+="import { assertAuditOpen } from './"+PREFIX+"-completion-guard.mjs';\n";
  prepend+=rule.guard.kind==='unconditional-prefix'?'assertAuditOpen();\n':'if('+JSON.stringify(rule.guard.modes)+'.includes(process.argv[2]))assertAuditOpen();\n';
 }
 const row={source:rule.source,destination:rule.destination,move:rule.source!==rule.destination,before_sha256:sha(raw),before_bytes:raw.length,role:rule.role,guard:rule.guard,patches,prepend,source_root_policy:'Current generic readers use the containing native checkout. Explicit frozen source readers, provenance records and old/active comparisons retain their historical roots. Audit output and helper dependencies resolve from tools.',preserved_roots:rule.preserved_roots};
 const output=Buffer.from(transform(before,row),'utf8');
 row.after_sha256=sha(output);row.after_bytes=output.length;
 return row;
});
const support_files=rules.support_files.map(rel=>({path:rel,sha256:sha(fs.readFileSync(safe(base,rel))),role:rel.endsWith('-prepare-migration.mjs')?'original-only plan writer':rel.endsWith('-relocated-validation.mjs')?'active syntax and safe-validator report writer':rel.endsWith('-migrate-helpers.mjs')?'active-only transactional migration runner':rel.endsWith('-migration-qa.mjs')?'original-only syntax QA writer':'pure support library'}));
const independent_review={source:'independent-review',destination:'evidence/independent-review',status:'unfinished independent review; no semantic certification claimed',change_policy:'Move the whole directory unchanged; do not modify findings or assert completion.',files:walkFiles(base,'independent-review').map(source=>({source,destination:'evidence/'+source,sha256:sha(fs.readFileSync(safe(base,source))),bytes:fs.statSync(safe(base,source)).size}))};
const summary={existing_scripts:scripts.length,support_files:support_files.length,total_helpers:scripts.length+support_files.length,moves:scripts.filter(r=>r.move).length,root_moves:scripts.filter(r=>r.move&&!r.source.includes('/')).length,evidence_moves:scripts.filter(r=>r.source.startsWith('evidence/')).length,changed_bodies:scripts.filter(r=>r.before_sha256!==r.after_sha256).length,exact_line_patches:scripts.reduce((n,r)=>n+r.patches.length,0),scripts_with_exact_line_patches:scripts.filter(r=>r.patches.length).length,new_completion_guards:scripts.filter(r=>r.guard.kind.endsWith('-prefix')).length,roles:scripts.reduce((a,r)=>(a[r.role]=(a[r.role]??0)+1,a),{})};
const plan={schema_version:1,captured_at:new Date().toISOString(),owner:'/root/helper_migration',status:mode==='--freeze'?'ready-for-active-copy':'draft-awaiting-coordinator-helper-freeze',source_only:true,application_or_test_execution:false,old_audit_root:OLD_AUDIT,active_audit_root:ACTIVE_AUDIT,reviewed_rules:{path:ruleRel,sha256:sha(ruleBytes)},summary,scripts,support_files,independent_review,preconditions:['Matching original source-only final gate passes with zero issues.','Coordinator freezes helper changes before the final plan and syntax QA hashes are bound.','The whole original audit is copied unchanged with /bin/cp -a and every copied file hash checked.','Application is restricted to the exact active copied audit and expected active HEAD.','Every script input, support module, import target and independent-review file matches the reviewed inventory and hashes.'],transaction:{runner:'tools/'+PREFIX+'-migrate-helpers.mjs',preview_argument:'--preview',apply_argument:'--apply',originals:'evidence/'+PREFIX+'-migration-originals.json',receipt:'evidence/'+PREFIX+'-migration-result.json',behavior:'Stage transformed helper bytes, recheck all inputs, then move/replace helpers and unchanged review directory. Any exception rolls back exact original helper bytes and review location.'},safe_validation:{preparation:'node docs/ice-gap/tools/'+PREFIX+'-migration-qa.mjs',active:'node docs/ice-gap/tools/'+PREFIX+'-relocated-validation.mjs',report:'evidence/relocated-helper-validation.json',methods:['Node --check only for every JavaScript helper.','/usr/bin/python3 -I with ast.parse from stdin for every Python helper; no compilation/cache writes.','Actual current read-only spec and structural validators plus visible-file local Git baseline comparison.','No historical writer, read-credit recorder, application module, test or build is executed.']},limitations:['Preparing a plan does not apply active migration.','Syntax success does not certify semantic correctness of historical authoring tools.','Old and active source-body evidence retains its existing attributed reviews and exclusions.','The independent reviewer remains unfinished; relocation does not alter that status.']};
fs.writeFileSync(safe(base,'evidence/'+PLAN_NAME,true),JSON.stringify(plan,null,2)+'\n');
console.log(JSON.stringify({path:'evidence/'+PLAN_NAME,sha256:sha(fs.readFileSync(safe(base,'evidence/'+PLAN_NAME))),status:plan.status,summary},null,2));
