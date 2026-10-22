// Preparation-only QA: parse the transformed helpers without executing them.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { OLD_AUDIT, PREFIX, PLAN_NAME, QA_NAME, sha, safe, readPreparation, syntaxPreparation } from './lane-exports-rpc-ai-auth-migration-core.mjs';
const base=path.dirname(path.dirname(fileURLToPath(import.meta.url)));
if(base!==OLD_AUDIT||fs.realpathSync(base)!==OLD_AUDIT||process.argv.length!==2)throw Error('Preparation QA runs only in the original audit tree without arguments.');
let report;
try{
 const prepared=readPreparation(base),syntax=syntaxPreparation(prepared),errors=syntax.filter(x=>!x.passed).map(x=>({path:x.path,error:x.error,stderr:x.stderr}));
 const scripts=prepared.scripts.map(s=>({source:s.row.source,destination:s.row.destination,before_sha256:s.row.before_sha256,after_sha256:s.after_sha256,changed:s.changed,move:s.row.move,role:s.row.role,guard:s.row.guard.kind,exact_replacements:s.row.patches.length}));
 report={schema_version:1,captured_at:new Date().toISOString(),owner:'/root/exports_rpc_ai_auth',passed:errors.length===0,syntax_only:true,application_or_test_execution:false,active_mutation_performed:false,plan_sha256:prepared.plan_sha256,source_root:base,node_version:process.version,summary:{scripts:scripts.length,support_files:prepared.support.length,moves:scripts.filter(s=>s.move).length,modified:scripts.filter(s=>s.changed).length,exact_replacements:scripts.reduce((n,s)=>n+s.exact_replacements,0),syntax_checks:syntax.length,syntax_failures:errors.length,new_completion_guards:scripts.filter(s=>s.guard.endsWith('prefix')).length},scripts,support_files:prepared.support.map(s=>({path:s.path,sha256:sha(s.raw)})),syntax_results:syntax,independent_review:prepared.plan.independent_review,checks:['Exact reviewed original hashes and exact-once line replacements match.','Move destinations are unique and absent for every relocated file.','Fresh script inventory has no unreviewed additions.','The reviewed current dependency form (single-line static relative imports) resolves to the explicit relocated/script support map; all input dependency forms were manually inspected.','The independent review has the exact two reviewed files and unchanged hashes.',errors.length===0?'Every transformed JavaScript file passed Node --check only; every Python file passed ast.parse only.':'Syntax parsing was attempted for every transformed script and support module; see failures in syntax_results.','No historical author, publisher, merger, read-credit recorder, application module, test or build was executed.'],errors,limits:['Syntax parsing does not execute or integration-test any helper.','This prepared artifact is not proof that active migration has run.','Original and active source-body evidence remains attributed to the existing audit artifacts; this work adds no source-read credit.']};
}catch(error){
 const p=safe(base,'evidence/'+PLAN_NAME);
 report={schema_version:1,captured_at:new Date().toISOString(),owner:'/root/exports_rpc_ai_auth',passed:false,syntax_only:true,application_or_test_execution:false,active_mutation_performed:false,plan_sha256:sha(fs.readFileSync(p)),source_root:base,errors:[{error:error.message}],scripts:[],syntax_results:[]};
}
fs.writeFileSync(safe(base,'evidence/'+QA_NAME,true),JSON.stringify(report,null,2)+'\n');
console.log(JSON.stringify({artifact:'evidence/'+QA_NAME,sha256:sha(fs.readFileSync(safe(base,'evidence/'+QA_NAME))),passed:report.passed,summary:report.summary,errors:report.errors},null,2));
if(!report.passed)process.exitCode=1;
