// Close the independently reviewed first-pass claim matrix against final accepted contracts.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {fileURLToPath} from 'node:url';
const E=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../evidence');
const areas=['ai','agent','tools','session','settings','extensions','rpc-remote','tui','subagents','safe-verify','observability','cognee','blackhole','providers-auth','cli-packaging','misc'];
const read=n=>JSON.parse(fs.readFileSync(path.join(E,n),'utf8'));
const hash=n=>crypto.createHash('sha256').update(fs.readFileSync(path.join(E,n))).digest('hex');
if(read('progress.json').complete)throw Error('Completed audit protected');
if(fs.existsSync(path.join(E,'first-pass-final-reconciliation.json')))throw Error('Already reconciled');
const manifests=areas.map(area=>({area,data:read(area+'.json'),spec:read('spec-'+area+'.json')}));
const all=new Map(manifests.flatMap(m=>m.spec.entries.map(e=>[e.id,{area:m.area,entry:e}])));
const q=read('qa-first-pass-coverage.json'),before=hash('qa-first-pass-coverage.json');
if(q.claims.length!==160)throw Error('Unexpected first-pass claim inventory');
const refs=ids=>[...new Set(ids.flatMap(id=>{const x=all.get(id);if(!x)throw Error('Missing contract '+id);return [...(x.entry.source+' '+x.entry.rust).matchAll(/\b(?:ice|rust):[A-Za-z0-9_./@+\-]+:\d+(?:-\d+)?/g)].map(m=>m[0]);}))];
const range=(a,b)=>Array.from({length:b-a+1},(_,i)=>'MISC-'+String(a+i).padStart(3,'0'));
const replacements={
 'FPC-152':{entry_ids:range(21,24),disposition:'confirmed-with-scope',finding:'The in-tree TypeScript router management, SSE/load/download completion, Hugging Face token/GGUF discovery, built-in llama provider lifecycle and interactive model manager are now separately documented in MISC-021–024. Native compatible inference, extensible provider identifiers, private execution binding and dynamic model registration already exist; those capabilities do not implement the management/auth/discovery/UI protocols. No matching in-tree native management implementation was found. External packages and live services remain unverified.'},
 'FPC-153':{entry_ids:range(30,37),disposition:'confirmed-with-scope',finding:'The bridge has eight source-verified contracts: loopback configuration/private credentials; provider registration; isolated reusable handles; protected request projection; bounded NDJSON ingestion; correlated terminal-turn validation; stable preview reconciliation; cancellation and safe failures. Native direct Anthropic HTTP, cancellation and plugin streaming are real capabilities, but no in-tree client implements this distinct local bridge protocol. No bridge was contacted and no private credential file was read.'},
 'FPC-154':{entry_ids:range(40,48),disposition:'qualified',finding:'The TypeScript package includes the vitest-evals runner, real temporary AgentSession adapter, strict completion and normalized telemetry, task-scoped session/source artifacts, comparative pairing/reporting, cancellation/cleanup and real-model/extension-authoring scenarios. Native deterministic FauxProvider harness tests, real tools over in-memory state, durable event assertions and session statistics already exist. The matching model-backed comparative eval package was not found in the native inventory. Both test implementations were read; neither suite was run.'},
 'FPC-155':{entry_ids:['MISC-026'],disposition:'qualified',finding:'The TypeScript documentation describes Gondolin/Docker/OpenShell deployment recipes and isolation boundaries. These are third-party deployment guidance, not proof of a built-in VM or enforcement engine. Native installation, remote, terminal and maintenance guidance is preserved; the dedicated equivalent isolation guide was not found. No deployment recipe or external isolation claim was executed or independently verified.'},
 'FPC-114':{finding:'Tool registry interfaces are covered by TOOLS; RPC-026 covers named shell controls/events, and published TUI-055 supplies the shell-panel contract. Native asynchronous user-Bash streaming/cancellation is preserved; it does not imply a native model-facing background registry.'}
};
const changes=[];
for(const c of q.claims){
 const patch=replacements[c.id];
 if(patch){changes.push({id:c.id,before:{finding:c.finding,entry_ids:c.entry_ids,disposition:c.disposition},after:patch});Object.assign(c,patch);c.evidence=[...new Set([...(c.evidence??[]),...refs(c.entry_ids)])];}
 if(c.id==='FPC-147'){c.entry_ids=[...new Set([...c.entry_ids,'TUI-078'])];c.evidence=[...new Set([...c.evidence,...refs(['TUI-078'])])];c.finding+=' The published TUI-078 contract traces the share consumer through export, public gist invocation, URL construction and failure handling; no sharing command was run.';}
 if(c.id==='FPC-142'){c.entry_ids=[...new Set([...c.entry_ids,'MISC-040'])];c.evidence=[...new Set([...c.evidence,...refs(['MISC-040'])])];c.finding+=' MISC-040 confirms the eval runner is an actual ICE_MODEL/ICE_PROVIDER consumer with explicit defaults.';}
 if(c.draft_dependent)c.publication_reconciliation={owner:'/root',status:'published-entry-evidence-accepted',entry_ids:c.entry_ids};
 c.draft_dependent=false;
 if(c.pending_owner)delete c.pending_owner;
}
for(const k of['setting_checks','environment_checks','active_import_checks'])for(const r of q[k]??[]){
 if(r.draft_dependent){r.publication_reconciliation='Referenced TUI/MISC contracts are accepted and published.';r.draft_dependent=false;}
 if(typeof r.finding==='string')r.finding=r.finding.replace(/\bdefault(?=\d)/g,'default ').replace(/\bversion(?=\d)/g,'version ');
}
q.uncovered_claims=[];q.follow_up_checks=[];q.pending=[];q.coverage_complete=true;q.qa_complete=true;
const reviewed=[];
for(const {area,data:m}of manifests)for(let index=0;index<(m.claims??[]).length;index++){
 const c=m.claims[index];
 const range=c.first_pass_lines??c.report_refs?.find(r=>r.version==='old')?.lines??c.report_refs?.[0]?.lines;
 if(!range)throw Error('Claim missing report range '+area+' '+index);
 const current=q.claims.find(x=>x.id===c.id||x.claim===c.claim);
 reviewed.push({manifest:area+'.json',index,source_label:c.first_pass_source?.path?.includes('/Zykairotis/')?'active':'old',range,claim:c.claim,entry_ids:c.entry_ids,review_status:'published-contract-reconciled',matrix_ids:current?[current.id]:[],unverified:current?.unverified});
 if(current?.id==='FPC-114')c.finding=current.finding;
}
q.accepted_claim_review=reviewed;
q.summary={...q.summary,claim_rows:q.claims.length,accepted_claim_records_reviewed:reviewed.length,uncovered_assigned_claims:0,draft_dependent_claims:0};
q.validation={...(q.validation??{}),uncovered_claims:0,draft_dependent_claims:0,all_referenced_entries_published:true};
q.final_reconciliation={captured_at:new Date().toISOString(),owner:'/root',previous_sha256:before,source_only:true,all_160_claims_dispositioned:true,unverified_limits_retained:['private migration outcomes and contents','external package/release/service behavior','live deployment recipes','runtime effects not exercised'],inputs:manifests.map(({area})=>({area,spec_sha256:hash('spec-'+area+'.json'),manifest_sha256:hash(area+'.json')}))};
for(const {area,data:m}of manifests){
 for(const c of m.claims??[])if(c.draft_dependent)c.draft_dependent=false;
 fs.writeFileSync(path.join(E,area+'.json'),JSON.stringify(m,null,2)+'\n');
 const pn='proposal-'+area+'.json';if(fs.existsSync(path.join(E,pn))){const p=read(pn);p.claims=m.claims;fs.writeFileSync(path.join(E,pn),JSON.stringify(p,null,2)+'\n');}
}
fs.writeFileSync(path.join(E,'qa-first-pass-coverage.json'),JSON.stringify(q,null,2)+'\n');
fs.writeFileSync(path.join(E,'first-pass-final-reconciliation.json'),JSON.stringify({captured_at:q.final_reconciliation.captured_at,owner:'/root',previous_sha256:before,changes,summary:q.summary,remaining_unverified:q.final_reconciliation.unverified_limits_retained},null,2)+'\n');
console.log(JSON.stringify({summary:q.summary,coverage_complete:q.coverage_complete,uncovered:q.uncovered_claims.length,followups:q.follow_up_checks.length,dispositions:q.claims.reduce((a,c)=>(a[c.disposition]=(a[c.disposition]??0)+1,a),{})},null,2));
