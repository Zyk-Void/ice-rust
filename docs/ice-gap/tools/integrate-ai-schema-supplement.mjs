// Targeted source-reviewed AI shared-schema addition; never executes application code.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {fileURLToPath} from 'node:url';
const E=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../evidence');
const read=n=>JSON.parse(fs.readFileSync(path.join(E,n),'utf8'));
const hash=n=>crypto.createHash('sha256').update(fs.readFileSync(path.join(E,n))).digest('hex');
if(read('progress.json').complete)throw Error('Completed audit is protected');
if(!fs.existsSync(path.join(E,'misc.json'))||!fs.existsSync(path.join(E,'tui.json')))throw Error('Wait for final MISC/TUI publication');
const spec=read('spec-ai.json'),fragment=read('spec-ai-schema-supplement.json'),p=read('proposal-ai-schema-supplement.json');
if(spec.entries.some(e=>e.id==='AI-019'))throw Error('AI-019 already integrated');
if(fragment.entries.length!==1||fragment.entries[0].id!=='AI-019')throw Error('Unexpected fragment');
if(hash('spec-ai-schema-supplement.json')!=='287c3f8942808164d537985109b92f720f02397eecb2ea1ab5353f8f0e86e8b6')throw Error('Unreviewed fragment change');
const repair=read('ai-schema-dependency-amendment.json').source_reference_repairs[0],e13=spec.entries.find(e=>e.id===repair.entry_id);
if(e13[repair.field]!==repair.old_field_value)throw Error('AI-013 anchor changed');
const before={spec_sha256:hash('spec-ai.json'),accepted_sha256:hash('ai.json'),proposal_sha256:hash('proposal-ai.json')};
e13[repair.field]=repair.new_field_value;
spec.entries.push(fragment.entries[0]);spec.entries.sort((a,b)=>a.id.localeCompare(b.id));
const qa={status:'reviewed',reviewer:'/root',source_only:true,entry_ids:['AI-019'],basis:['evidence/ai-schema-source-review.json','evidence/ai-schema-qa.json','evidence/coordinator-ai-schema-review.json'],notes:'Full worker source review and independent bounded coordinator review. ThinkingLevelMap includes off; Model.compat conditional is explicit; timestamp units are Unix milliseconds. Raw duplicate-role candidate remains unverified as a production failure.'};
const writes=[];
for(const name of['ai.json','proposal-ai.json']){
 const m=read(name);m.entries.push(p.entries[0]);m.entries.sort((a,b)=>a.id.localeCompare(b.id));
 for(const u of p.ledger_updates){
  const row=m.ledger.find(r=>r.path===u.path);if(!row)throw Error('AI ledger row absent');
  row.entry_ids=[...new Set([...row.entry_ids,...u.entry_ids_to_add])];
  row.notes+=' Shared nested schemas additionally documented in AI-019; existing read lines are not counted twice.';
 }
 m.surfaces.push(...p.surfaces);
 for(const r of p.read_evidence)m.read_evidence.push({path:r.repository+':'+r.path,sha256:r.sha256,total_lines:r.total_lines,ranges:r.read_ranges,reader:'audit_tools',notes:r.read_method,baseline:r.baseline});
 m.targeted_supplements=[...(m.targeted_supplements??[]),{entry_id:'AI-019',fragment_sha256:hash('spec-ai-schema-supplement.json'),independent_qa:qa}];
 writes.push([name,m]);
}
const defects=read('rust-defects.json'),d=defects.find(d=>d.id==='RUST-035');if(!d)throw Error('Existing stopReason defect absent');
d.entry_ids=[...new Set([...d.entry_ids,'AI-019'])];
const decision={captured_at:new Date().toISOString(),owner:'/root',source_only:true,before,after_entries:spec.entries.length,added_entry:'AI-019',added_surfaces:p.surfaces.length,source_anchor_repair:repair,independent_qa:qa,existing_defect_link:{id:'RUST-035',entry_id:'AI-019'},raw_role_candidate:{source:'evidence/ai-schema-defect-candidates.json',id:'AI-SCHEMA-RAW-ROLE',decision:'retain-bounded-candidate-without-new-defect-id',reason:'Source structure independently confirmed. No fully traced raw production caller or runtime reproduction. The current RPC Value projection is not asserted to expose duplicate role keys.'}};
fs.writeFileSync(path.join(E,'spec-ai.json'),JSON.stringify(spec,null,2)+'\n');
for(const[name,data]of writes)fs.writeFileSync(path.join(E,name),JSON.stringify(data,null,2)+'\n');
fs.writeFileSync(path.join(E,'rust-defects.json'),JSON.stringify(defects,null,2)+'\n');
fs.writeFileSync(path.join(E,'ai-schema-coordinator-integration.json'),JSON.stringify(decision,null,2)+'\n');
console.log(JSON.stringify({entry_count:spec.entries.length,added:'AI-019',surface_additions:p.surfaces.length,existing_defect:'RUST-035',new_defects:0},null,2));
