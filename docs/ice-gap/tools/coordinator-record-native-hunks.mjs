import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
assertAuditOpen();
// Persist the coordinator's completed old/active hunk review. No application execution.
import fs from 'node:fs';import path from 'node:path';import crypto from 'node:crypto';import{fileURLToPath}from'node:url';
const E=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../evidence');
const input='active-checkout-diff-hunks-complete.json',raw=fs.readFileSync(path.join(E,input)),map=JSON.parse(raw);
const sha=x=>crypto.createHash('sha256').update(x).digest('hex');
const decisions={
 'brand.rs':{behavior:[20,62,64,103,107],note:'NO_EMOJI_ENV now looks up ICERS_NO_EMOJI; wordmark/title output says icers. Layout and toggle logic are unchanged.'},
 'changelog.rs':{behavior:[27,31,227],note:'Repository owner is lowercase; override is ICERS_CHANGELOG_URL; HTTP User-Agent is icers/version. RUST-045 is fixed at active line27; release selection and marker logic unchanged.'},
 'config.rs':{behavior:[43,47,1007],note:'CONFIG_DIR_NAME=.icers; CONFIG_DIR_ENV=ICERS_CODING_AGENT_DIR; atomic filename fallback becomes icers. The existing flat-to-nested migration stays within the selected native config root; old TS .ice paths must not be renamed in audit prose.'},
 'prebuilt_release.rs':{behavior:[11,76,78,81,101,105],note:'Repository constant lowercase fixes RUST-045. Manifest flat executable identity is icers[.exe]; HTTP/output identity follows. Archive validation/security predicates and crate manifest name ice-cli are unchanged.'},
 'resource_dirs.rs':{behavior:[43],note:'PROJECT_CONFIG_DIR_NAME=.icers selects native project resources. The test change at old431 adds two formatting lines while asserting the renamed paths. Precedence and deduplication behavior is unchanged.'},
 'session.rs':{behavior:[96,104,124,191,1940,2658],note:'System prompt/output identity, native project sessions directory and ICERS_BASH_TIMEOUT change. Host session identity is ICERS_SESSION_ID through SDK constant. Tool construction/trust/locking conditions are unchanged.'},
 'settings.rs':{behavior:[395,507],note:'Project settings read/write paths are .icers/settings.json. Global path follows config constants. Persisted setting names and precedence logic are unchanged.'},
 'updates.rs':{note:'Renamed runtime/output identities include icers executable, ICERS_DISABLE_UPDATE_CHECK and freshness.icers. Public/private helper names containing ice and crates.io ice-cli remain. STAGED_ICERS_CHECK_TIMEOUT stays10seconds and MAX_STAGED_ICERS_OUTPUT_BYTES stays4096. Added lines are string formatting only; version parser uses APP_NAME/SDK HOST_NAME, so the required prefix follows icers.'}
};
const reviews=[];
for(const[file,decision]of Object.entries(decisions)){
 const rel='crates/ice-cli/src/'+file,r=map.rows.find(x=>x.old_path===rel);if(!r)throw Error(rel);
 const pieces=r.unified_diff.split(/(?=^@@ )/m).filter(x=>x.startsWith('@@ '));
 if(pieces.length!==r.changes.length)throw Error('Expected U0 one segment per hunk '+rel);
 const hunks=pieces.map((body,i)=>{
  const c=r.changes[i],lines=body.split('\n').slice(1),old=lines.filter(x=>x.startsWith('-')).map(x=>x.slice(1)),active=lines.filter(x=>x.startsWith('+')).map(x=>x.slice(1));
  let behavioral=decision.behavior?.includes(c.old_start)??(file==='updates.rs'&&c.old_start<2400&&!old.every(x=>x.trim().startsWith('//'))&&c.old_start!==22);
  let rationale=decision.note;
  if(file==='updates.rs'&&[79,255,336,815].includes(c.old_start))rationale='Serialized freshness key/access changes ice to icers. Active UpdateCacheFreshness.icers has no field default or legacy alias; old freshness objects may fail deserialization if reused. This is recorded as a bounded cache-compatibility consequence, not assumed migration support.';
  if(file==='updates.rs'&&c.old_start===22)rationale='Private constant identifiers are renamed; numeric values remain10seconds and4*1024bytes. Their uses are updated at old1889/active1893.';
  return{index:i,...c,old_lines:old,active_lines:active,classification:behavioral?'behavior':'identity-only',formatting_changed:c.old_count!==c.active_count,confirmed:true,rationale,reviewer:'/root'};
 });
 reviews.push({old_path:r.old_path,active_path:r.active_path,old_sha256:r.old.sha256,active_sha256:r.active.sha256,diff_sha256:sha(r.unified_diff),hunks,confirmed:true});
}
const report={schema_version:1,captured_at:new Date().toISOString(),source_map:input,source_map_sha256:sha(raw),old_head:'ff9270f26e263ea9ce54349786bf2d93c4517476',active_head:'1891337558105fb8322c79617e5a03d98c1b2c3a',method:'Coordinator manually read the complete saved old/active unified diff for each of these eight files. This records reviewed changed lines only; unchanged ranges retain previously certified credit and exact offset transfer.',files:reviews,file_count:reviews.length,hunk_count:reviews.reduce((n,r)=>n+r.hunks.length,0),unreviewed:0,connector_redacted_changed_lines:0,source_execution:false,runtime_verified:false,followup_active_reads:[{path:'crates/ice-cli/src/updates.rs',sha256:'587fdaa1fbae831826bea57eb1409cd5a884297a713d0cce57830e84f6cc6470',ranges:[[48,90],[247,267],[276,348],[635,653],[1921,1941]]},{path:'crates/ice-cli/src/lib.rs',sha256:'c1bd647b4f6730bf403899e5d326d0c5aa4c0d654bc21f013542b7d218efb0ba',ranges:[[1,90]]}]};
const out='coordinator-native-hunk-review.json';fs.writeFileSync(path.join(E,out),JSON.stringify(report,null,2)+'\n');
console.log(JSON.stringify({file:out,files:report.file_count,hunks:report.hunk_count,behavior:reviews.flatMap(r=>r.hunks).filter(h=>h.classification==='behavior').length,identity_only:reviews.flatMap(r=>r.hunks).filter(h=>h.classification==='identity-only').length,sha256:sha(fs.readFileSync(path.join(E,out)))},null,2));
