import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
assertAuditOpen();
// Apply independently reviewed first-pass metadata repairs, preserving original records.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {fileURLToPath} from 'node:url';
const base=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const E=path.join(base,'evidence');
const read=n=>JSON.parse(fs.readFileSync(path.join(E,n),'utf8'));
const hash=n=>crypto.createHash('sha256').update(fs.readFileSync(path.join(E,n))).digest('hex');
const repair=read('qa-first-pass-repairs.json');
const files=new Map();
const originals=[];
function get(name,expected){if(!files.has(name)){if(expected&&hash(name)!==expected)throw Error('Unexpected reviewed manifest '+name);const data=read(name);files.set(name,data);originals.push({path:'evidence/'+name,sha256:hash(name),claims:data.claims});}return files.get(name);}
let black=get('blackhole.json',repair.blackhole.expected_sha256);
for(const r of repair.blackhole.record_repairs){const old=black.claims[r.index];if(old.claim!==r.original_claim||JSON.stringify(old.first_pass_lines)!==JSON.stringify(r.original_range))throw Error('Blackhole repair no longer applies '+r.index);}
black.claims=repair.blackhole.replacement_claims;
black.additional_audit_behavior_claims=repair.blackhole.record_repairs.filter(r=>r.action==='reclassify_as_additional_audit_behavior').map(r=>({original_claim:r.original_claim,reason:r.reason,entry_ids:originals[0].claims[r.index].entry_ids}));
for(const r of repair.source_label_repairs){const data=get(r.manifest,r.expected_sha256),old=data.claims[r.index];if(old.claim!==r.original_claim)throw Error('Source-label repair no longer applies '+r.manifest);old.first_pass_source=r.first_pass_source;old.first_pass_lines=r.first_pass_lines;}
const tm=get('tools.json','111865d85e2a3625d6f0f273bc256f2cd13b94dab8e0d58b3abafb174deea736');const tc=tm.claims[4];if(tc.claim!=='Shells panel and RPC background controls/events.')throw Error('Tool claim moved');
tc.first_pass_source={path:'/home/mewtwo/Zks/ice-rust/ICE_PORTING.md',sha256:'d66e4d33ef0023d228110786f03aeb73c97e72df454b1e481f375b3ac264d7fc'};
tc.verdict='confirmed-with-cross-area-scope';
tc.finding='The tool registry and session delivery contracts are documented in TOOLS and SESSION. RPC-026 verifies command responses and bash background/watch events. TUI-055 is a source-reviewed draft for the shells panel and must be published before global completion; no runtime behavior was exercised.';
tc.entry_ids=repair.extra_cross_area_repair.entry_ids;
tc.evidence=[...new Set([...tc.evidence,...read('rpc-remote.json').claims.flatMap(c=>c.entry_ids.includes('RPC-026')?c.evidence:[])])];
const pendingTui=read('spec-tui.json').entries.find(e=>e.id==='TUI-055');if(!pendingTui)throw Error('Expected TUI contract missing');
const refs=[...pendingTui.source.matchAll(/\b(?:ice|rust):[A-Za-z0-9_./@+\-]+:\d+(?:-\d+)?/g)].map(m=>m[0]);tc.evidence=[...new Set([...tc.evidence,...refs])];
const history={captured_at:new Date().toISOString(),source_only:true,repair_artifact:'evidence/qa-first-pass-repairs.json',originals,notes:'Claim metadata only; no detailed gap entry removed or rewritten. Canonical Blackhole first-pass records are eight; four additional-audit behaviors are retained outside the first-pass count.'};
const output=n=>path.join(E,n);
fs.writeFileSync(output('first-pass-records-before-reconciliation.json'),JSON.stringify(history,null,2)+'\n');
for(const[name,data]of files){fs.writeFileSync(output(name),JSON.stringify(data,null,2)+'\n');const pn='proposal-'+name;if(fs.existsSync(output(pn))){const proposal=read(pn);proposal.claims=data.claims;if(data.additional_audit_behavior_claims)proposal.additional_audit_behavior_claims=data.additional_audit_behavior_claims;fs.writeFileSync(output(pn),JSON.stringify(proposal,null,2)+'\n');}}
console.log(JSON.stringify({updated:[...files.keys()].map(name=>({name,sha256:hash(name),claims:files.get(name).claims.length})),history:'evidence/first-pass-records-before-reconciliation.json',pending:'TUI-055 publication and full first-pass matrix reconciliation'},null,2));
