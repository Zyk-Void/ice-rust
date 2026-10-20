import fs from 'node:fs';import path from 'node:path';import{createHash}from'node:crypto';import{fileURLToPath}from'node:url';
const base=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..'),ev=path.join(base,'evidence'),p=path.join(ev,'lane-final-areas-read-coverage-proposal.json'),q=path.join(ev,'lane-final-areas-read-coverage-qa.json'),o=JSON.parse(fs.readFileSync(p,'utf8')),qa=JSON.parse(fs.readFileSync(q,'utf8')),hash=b=>createHash('sha256').update(b).digest('hex'),errors=[],warnings=[];
function csv(text){const rows=[];let row=[],cell='',quoted=false;for(let i=0;i<text.length;i++){const c=text[i];if(c==='"'){if(quoted&&text[i+1]==='"'){cell+='"';i++}else quoted=!quoted}else if(c===','&&!quoted){row.push(cell);cell=''}else if(c==='\n'&&!quoted){row.push(cell.replace(/\r$/,''));rows.push(row);row=[];cell=''}else cell+=c}if(cell||row.length){row.push(cell);rows.push(row)}const h=rows.shift();return rows.filter(r=>r.length>1).map(r=>Object.fromEntries(h.map((k,i)=>[k,r[i]||''])))}
function spans(ns){const out=[];for(const n of ns){if(out.length&&n===out.at(-1)[1]+1)out.at(-1)[1]=n;else out.push([n,n])}return out}
function physical(text){if(!text)return 0;const xs=text.split(/\r\n|[\n\r\v\f\x1c-\x1e\x85\u2028\u2029]/);return xs.length-(/[\n\r\v\f\x1c-\x1e\x85\u2028\u2029]$/.test(text)?1:0)}
const metadata=new Map(),gaps=[],exclusionLineTotal={redacted:0,unverified:0};
for(const[k,r]of Object.entries(o.files)){
 const i=k.indexOf(':'),side=k.slice(0,i),rel=k.slice(i+1),b=fs.readFileSync(path.join(o.source_roots[side],rel)),text=b.toString('utf8'),h=hash(b),count=physical(text);metadata.set(k,{sha256:h,lines:count});
 if(r.sha256!==h)errors.push({kind:'current_hash_mismatch',path:k,sha256:r.sha256,current:h});
 if(r.total_lines!==count)errors.push({kind:'physical_line_count_mismatch',path:k,record:r.total_lines,current:count});
 const sets={redacted:new Set(r.excluded_redacted_lines),unverified:new Set(r.excluded_unverified_lines)},covered=new Set();
 for(const[a,z]of r.ranges){if(!Number.isInteger(a)||!Number.isInteger(z)||a<1||z<a||z>count)errors.push({kind:'invalid_credited_range',path:k,range:[a,z],count});for(let n=a;n<=z;n++){if(covered.has(n))errors.push({kind:'range_overlap',path:k,line:n});covered.add(n)}}
 for(const kind of ['redacted','unverified'])for(const n of sets[kind]){exclusionLineTotal[kind]++;if(n<1||n>count)errors.push({kind:'invalid_exclusion',path:k,line:n,count});if(covered.has(n))errors.push({kind:'exclusion_has_read_credit',path:k,line:n});if(kind==='redacted'&&sets.unverified.has(n))errors.push({kind:'ambiguous_exclusion_class',path:k,line:n})}
 const excluded=new Set([...sets.redacted,...sets.unverified]),missing=Array.from({length:count},(_,i)=>i+1).filter(n=>!covered.has(n)&&!excluded.has(n));
 if(missing.length)gaps.push({path:k,ordinary_missing_ranges:spans(missing),missing_lines:missing.length,total_lines:count});
 if(r.visible_lines!==covered.size)errors.push({kind:'visible_count_mismatch',path:k});
 if(r.fully_read!==(!missing.length&&!excluded.size))errors.push({kind:'fully_read_flag_mismatch',path:k});
}
const policyDispositions=[];
const ledger=csv(fs.readFileSync(path.join(base,'ledger.csv'),'utf8')),ledgerResult=[];
for(const r of ledger){if(r.status!=='mapped')continue;const k=r.path.startsWith('ice:')||r.path.startsWith('rust:')?r.path:'ice:'+r.path,record=o.files[k];
 if(!record){errors.push({kind:'missing_mapped_ledger_read',path:k});continue}
 const m=metadata.get(k),rowGap=gaps.find(x=>x.path===k);
 if(rowGap){
 const policy=o.coverage_policies.find(x=>x.path===k&&x.kind==='generated-data-group');
 if(policy&&JSON.stringify(policy.not_read_ranges)===JSON.stringify(rowGap.ordinary_missing_ranges)&&policy.generator.fully_read&&policy.consumer.fully_read&&policy.sha256===record.sha256&&hash(fs.readFileSync(path.join(base,'requirements.md')))===policy.rule.sha256&&policy.rule.line===23){
  policyDispositions.push({path:k,kind:policy.kind,ordinary_missing_ranges:rowGap.ordinary_missing_ranges,rule:policy.rule,evidence:policy.evidence,visible_read_credit_unchanged:true});
 }else errors.push({kind:'unread_required_source',...rowGap});
 }
 ledgerResult.push({path:k,physical_lines:m.lines,visible_lines:record.visible_lines,excluded_redacted_lines:record.excluded_redacted_lines,excluded_unverified_lines:record.excluded_unverified_lines,ordinary_missing_ranges:rowGap?.ordinary_missing_ranges||[],policy_disposition:policyDispositions.find(x=>x.path===k)?.kind||null});
}
const old=JSON.parse(fs.readFileSync(path.join(ev,'read-coverage.json'),'utf8')),oldfiles=old.files||old;
for(const k of Object.keys(oldfiles).filter(k=>k.startsWith('ice:')||k.startsWith('rust:')))if(!o.files[k])errors.push({kind:'lost_legacy_record',path:k});
if(!o.files['rust:crates/ice-cli/src/interactive_tui/sessions.rs'].excluded_unverified_lines.includes(627))errors.push({kind:'lost_sessions_627_limit'});
if(o.baseline_identity_limitations.length!==5)errors.push({kind:'dirty_baseline_gap_count',actual:o.baseline_identity_limitations.length});
if(qa.unmatched_exclusions.length)errors.push({kind:'unmatched_exclusions',paths:qa.unmatched_exclusions.map(x=>x.path)});
for(const x of qa.copy_identity_errors||[])errors.push({kind:'copy_identity_error',...x});
for(const x of qa.source_errors)errors.push({kind:'source_error',...x});
for(const x of qa.malformed_inputs)errors.push({kind:'malformed_input',...x});
for(const x of qa.identity_conflicts)errors.push({kind:'identity_conflict',...x});
qa.status=errors.length?'FAIL':'PASS';qa.proposal_sha256=hash(fs.readFileSync(p));qa.validation={method:'Static JSON/CSV/range/hash evidence checks only; no application code, build or tests executed',errors,warnings,current_source_files_verified:metadata.size,canonical_read_hash_gaps:Object.values(o.files).filter(r=>!r.sha256).length,ledger_rows:ledger.length,mapped_ledger_rows:ledgerResult.length,ledger_ordinary_missing_files:ledgerResult.filter(r=>r.ordinary_missing_ranges.length&&!r.policy_disposition).length,policy_dispositions:policyDispositions,derived_copy_files:o.identical_copy_proofs.length,exclusion_line_totals:exclusionLineTotal,legacy_records_preserved:Object.keys(oldfiles).filter(k=>o.files[k]).length,ordinary_partial_support_files:gaps,ledger_coverage:ledgerResult};
fs.writeFileSync(q,JSON.stringify(qa,null,2)+'\n');
console.log(JSON.stringify({status:qa.status,proposal_sha256:qa.proposal_sha256,summary:o.summary,validation:{...qa.validation,ledger_coverage:undefined},rust_non_crates:o.rust_non_crates}));
