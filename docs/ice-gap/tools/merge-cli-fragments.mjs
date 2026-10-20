import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
assertAuditOpen();
// Merge immutable CLI audit fragments only; no application execution.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {fileURLToPath} from 'node:url';
const base=path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const read=n=>JSON.parse(fs.readFileSync(path.join(base,'evidence',n),'utf8'));
const hash=n=>crypto.createHash('sha256').update(fs.readFileSync(path.join(base,'evidence',n))).digest('hex');
const expected={
 'spec-cli-packaging.json':'6c95df2d47ff90b42f692fdc540fcaadc4342ccc5aa3aee4e2b4c90e1733e501',
 'spec-cli-packaging-supplement.json':'5b80c1489cb7fb035182c92f683ad20bf5492841c8818c2a7ca20efa25ee5f80',
 'proposal-cli-packaging-supplement.json':'73481d73b0a8ff4c710d893b0604d9489366b370e91dd73104e032b09d336b8d'
};
for(const [n,h] of Object.entries(expected))if(hash(n)!==h)throw Error('Fragment changed; review before merge: '+n);
const s=read('spec-cli-packaging.json'), t=read('spec-cli-packaging-supplement.json');
const p=read('proposal-cli-packaging.json'),q=read('proposal-cli-packaging-supplement.json');
const unique=(xs,key)=>{const m=new Map();for(const x of xs){const k=key(x);if(m.has(k)){if(JSON.stringify(m.get(k))!==JSON.stringify(x))throw Error('Conflicting '+k);}else m.set(k,x);}return [...m.values()];};
const entries=unique([...s.entries,...t.entries],e=>e.id).sort((a,b)=>a.id.localeCompare(b.id));
const spec={...s,title:'CLI, packaging and startup',introduction:'Complete source-derived CLI and packaging specification, including startup, credential commands, migrations, npm/Git/local resource backends, native Cargo/prebuilt preservation and updater defects. No application, test, installer or updater was executed. Final global and active-checkout reconciliation remain pending.',method:[s.method,t.method].filter(Boolean).join('\n\n'),entries,draft_complete:true,complete:false,fragments_merged:true};
const combined={...p,area:'cli-packaging',draft_complete:true,complete:false,entries:unique([...p.entries,...q.entries],e=>e.id).sort((a,b)=>a.id.localeCompare(b.id)),fragment_sources:Object.fromEntries(['spec-cli-packaging.json','proposal-cli-packaging.json','spec-cli-packaging-supplement.json','proposal-cli-packaging-supplement.json'].map(n=>[n,hash(n)]))};
for(const k of ['ledger','surfaces','read_evidence','candidate_exclusions','claims','dependency_requests','rust_only','open_questions','supplemental_ledger'])combined[k]=[...(p[k]??[]),...(q[k]??[])];
const ledgerMap=new Map();for(const row of combined.ledger){const prior=ledgerMap.get(row.path);if(!prior||prior.status==='pending'){ledgerMap.set(row.path,row);}else if(row.status==='pending')continue;else if(JSON.stringify(prior)!==JSON.stringify(row))throw Error('Conflicting resolved ledger '+row.path);}combined.ledger=[...ledgerMap.values()];
combined.candidate_exclusions=unique(combined.candidate_exclusions,x=>x.candidate_id);
combined.surfaces=unique(combined.surfaces,x=>[x.category,x.name,x.ice_ref,x.entry_id].join('\0'));
const merge=rs=>{const out=[];for(const r of rs.sort((a,b)=>a[0]-b[0]||a[1]-b[1])){const prev=out.at(-1);if(prev&&r[0]<=prev[1]+1)prev[1]=Math.max(prev[1],r[1]);else out.push([...r]);}return out;};
const records=new Map();
for(const r of combined.read_evidence){if(!records.has(r.path))records.set(r.path,{...r,ranges:[],excluded_redacted_lines:[],attribution:[]});const n=records.get(r.path);if(n.sha256&&r.sha256&&n.sha256!==r.sha256)throw Error('Read hash mismatch '+r.path);n.sha256??=r.sha256;n.total_lines=Math.max(n.total_lines??0,r.total_lines??0);n.ranges.push(...(r.ranges??[]));n.excluded_redacted_lines.push(...(r.excluded_redacted_lines??r.redacted_lines??[]));n.attribution.push({owner:r.owner??'source-review-fragment',notes:r.notes??'',ranges:r.ranges??[]});}
combined.read_evidence=[...records.values()].map(r=>{r.ranges=merge(r.ranges);r.excluded_redacted_lines=[...new Set(r.excluded_redacted_lines)].sort((a,b)=>a-b);for(const x of r.excluded_redacted_lines)r.ranges=r.ranges.flatMap(([a,b])=>x<a||x>b?[[a,b]]:[[a,x-1],[x+1,b]].filter(([c,d])=>c<=d));r.fully_read=r.total_lines>0&&r.excluded_redacted_lines.length===0&&r.ranges.length===1&&r.ranges[0][0]===1&&r.ranges[0][1]>=r.total_lines;return r;});
combined.comparison_notes=[...(p.comparison_notes??[]),...(q.comparison_notes??[])];
combined.limitations=[...(p.limitations??[]),...(q.limitations??[])];
combined.independent_qa=['evidence/qa-cli-coordinator.json','evidence/cli-packaging-supplement-qa.json'];
combined.qa={startup:p.qa??null,package_supplement:q.qa??null,global_complete:false};
for(const [source,target]of [['spec-cli-packaging.json','spec-cli-packaging-startup-fragment.json'],['proposal-cli-packaging.json','proposal-cli-packaging-startup-fragment.json']]){const f=path.join(base,'evidence',target);if(fs.existsSync(f))throw Error('Refusing fragment snapshot overwrite '+target);fs.copyFileSync(path.join(base,'evidence',source),f);}
for(const[n,x]of [['spec-cli-packaging.json',spec],['proposal-cli-packaging.json',combined]])fs.writeFileSync(path.join(base,'evidence',n),JSON.stringify(x,null,2)+'\n');
console.log(JSON.stringify({entries:entries.length,entry_ids:entries.map(e=>e.id),ledger:combined.ledger,surfaces:combined.surfaces.length,reads:combined.read_evidence.length,full_reads:combined.read_evidence.filter(r=>r.fully_read).length,excluded_redactions:combined.read_evidence.filter(r=>r.excluded_redacted_lines.length).map(r=>({path:r.path,lines:r.excluded_redacted_lines})),candidate_exclusions:combined.candidate_exclusions,claims:combined.claims.length,spec_sha256:hash('spec-cli-packaging.json'),proposal_sha256:hash('proposal-cli-packaging.json')},null,2));
