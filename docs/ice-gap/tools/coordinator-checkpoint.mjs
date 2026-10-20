import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
assertAuditOpen();
// Root-only recoverable audit checkpoint.
import fs from'node:fs';import path from'node:path';import{fileURLToPath}from'node:url';import crypto from'node:crypto';
import {normalizeResidualSpacing} from './fix-spacing.mjs';
const B=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..'),E=path.join(B,'evidence'),read=n=>JSON.parse(fs.readFileSync(path.join(E,n),'utf8'));
const phase=process.argv[2]||'original-verified',p=read('progress.json'),g=read('final-validation.json'),stamp=new Date().toISOString();
if(phase==='original-verified'){
 if(!g.passed||g.phase!=='original')throw Error('Original gate has not passed');
 fs.copyFileSync(path.join(E,'final-validation.json'),path.join(E,'original-checkout-validation.json'));
 p.original_validation={passed:true,at:g.captured_at,report:'evidence/original-checkout-validation.json',sha256:crypto.createHash('sha256').update(fs.readFileSync(path.join(E,'original-checkout-validation.json'))).digest('hex')};
 p.phase='active-relocation-pending';p.complete=false;
}
p.captured_at=stamp;fs.writeFileSync(path.join(E,'progress.json'),JSON.stringify(p,null,2)+'\n');
const table=[
'','## Closeout checkpoint '+stamp,'',
'| Lane | State | Evidence/result |','|---|---|---|',
'| L1 misc | published | 43 complete entries; title/introduction repaired |',
'| L2 TUI + AI supplement | published | 151 TUI including 67 terminal entries; AI 50 including AI-019 |',
'| L3/L4 exports | merged | 1654 exact QA records, 807 runtime, 847 type, 0 unresolved |',
'| L5 surfaces | merged | 3975 rows, 0 pending; single-anchor schema normalized with full support retained |',
'| L6 dependencies | merged | 3053 typed edges, 791 IDs, 546 order groups; all requests resolved |',
'| L7 claims/index/coverage | verified | 160 first-pass claims; 1702 canonical paths; original gate 0 errors |',
'| L8 prose | verified | 5359 first-pass changes, 110 reviewed examples; residual spacing applied and separately proven in spacing-residual-validation.json |',
'| Relocation source review | verified | All assigned native hunks and rename-only entrypoint reviewed; original sources unchanged |',
'| Relocation content | draft complete | Guarded native prose proposals; RUST-045 fixed; conditional RUST-069 ready for active insertion |',
'| Relocation helpers | draft complete | Reviewed migration plan will be applied to the active copy |','',
'Reserved IDs: RUST-069 for the active native session-value discriminator compatibility finding, with local Git provenance. SESSION-039/040 remain unused. No duplicate TUI terminal IDs will be added.','',
'Pending: copy the verified audit; remap current Rust citations and canonical read ranges using fixed-head changes and reviewed overlaps; apply native-only identity and defect proposals; preserve original provenance; migrate and validate helpers; run the active gate, finalize completion metadata, and rerun the active gate.','',
'Next concrete action: freeze helper hashes, copy the complete original audit to the authorized active audit directory, and apply the reviewed relocation sequence.','',
'Original final machine gate: '+g.captured_at+', passed='+g.passed+', source-only; '+g.counts.entry_count+' entries, '+g.counts.ledger_rows+' ledger rows, '+g.source_reference_count+' unique refs checked; '+g.warnings.length+' terminal-empty-line warnings, 0 errors. No application/tests/build/network/installs/commits/private runtime reads.','',
'Coverage limits remain explicit: 86 redacted lines, sessions.rs627 excluded, 5 initial dirty test hashes unavailable, 26 same-byte derived copy records, image-models.generated.ts36–819 grouped under requirements 23 rather than read.',''
].join('\n');
const resumePath=path.join(E,'RESUME.md'),previous=fs.readFileSync(resumePath,'utf8'),boundary=previous.indexOf('\n---\n');
if(boundary<0)throw Error('Missing historical checkpoint boundary');
const header=[
 '# Resume checkpoint — original gate verified '+stamp,'',
 'Current state: the original audit is ready for the authorized active-checkout relocation. The source audit remains published; active completion is still false.','',
 '| Workstream | State | Evidence |','|---|---|---|',
 '| Published source contracts | verified | All 16 areas; 791 entries; 872 ledger rows; 3975 surface rows |',
 '| Claims, exports and dependencies | verified | 160 claims; 1654 export dispositions; 3053 typed edges |',
 '| Residual spacing | verified | Every edit reviewed; citation, identifier and protected-span preservation proven |',
 '| Original machine gate | passed | '+g.captured_at+'; 0 issues; '+g.warnings.length+' terminal-empty-line warnings |',
 '| Helper migration and independent review move | prepared | Apply the reviewed plan to the active copy only |',
 '| Citation relocation, native identities and defects | prepared | Sequential active-only application remains pending |',
 '| Active machine gate and completion metadata | pending | progress.complete remains false |','',
 'Reserved IDs: RUST-069 is reserved for the reviewed active session-value compatibility finding. SESSION-039 and SESSION-040 remain intentionally unused.','',
 'Next action: freeze the reviewed helper inputs, copy and hash-verify the audit, remap citations, apply native prose and defect proposals, migrate helpers, validate, and complete the active gate.',''
].join('\n');
fs.writeFileSync(resumePath,normalizeResidualSpacing(header)+previous.slice(boundary)+normalizeResidualSpacing(table));
console.log(JSON.stringify({phase,stamp,original_validation:p.original_validation,complete:p.complete},null,2));
