import { auditHelperBase } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
assertAuditOpen();
// Finish only the source-verified Blackhole area. No inferred parity mappings.
import fs from 'node:fs';
import path from 'node:path';
const root='/home/mewtwo/Zks/ice';
const base=auditHelperBase;
const prefix='packages/coding-agent/examples/extensions/ice-blackhole/';
const load=p=>JSON.parse(fs.readFileSync(path.join(base,p),'utf8'));
const spec=load('evidence/spec-blackhole.json');
const candidates=load('evidence/candidates-blackhole.json');
const original=load('evidence/tracked-files.json').filter(r=>r.area==='blackhole');
const copies=load('evidence/blackhole-copies.json');
const id=n=>'BLACK-'+String(n).padStart(3,'0');
const mapping={
 'LICENSE':[1],'PROVENANCE.md':[1,2,41],'package.json':[1],'index.ts':[1,5,6],
 'src/core/brief.ts':[13,14,15,16,17],'src/core/build-sections.ts':[18,19,20,21,22,24,25,26],
 'src/core/checkpoint-recall.ts':[37],'src/core/content.ts':[10,11],
 'src/core/filter-noise.ts':[9],'src/core/format.ts':[26],'src/core/live-state.ts':[32,33,35,36],
 'src/core/normalize.ts':[7],'src/core/refine.ts':[38],'src/core/sanitize.ts':[8],
 'src/core/skill-collapse.ts':[12],'src/core/subagents.ts':[34],'src/core/summarize.ts':[27,28,29,30],
 'src/core/tail.ts':[31],'src/core/tool-args.ts':[11,15],'src/core/unified-config.ts':[2,3,4],
 'src/extract/commits.ts':[23],'src/extract/files.ts':[22],'src/extract/goals.ts':[24],
 'src/extract/preferences.ts':[25],'src/hooks/before-compact.ts':[39],'src/hooks/trim-kept-tail.ts':[40],
 'src/om/compaction-trigger.ts':[41],'src/sections.ts':[26],'src/types.ts':[7,22]
};
const ledger=original.map(r=>{const ns=mapping[r.path.slice(prefix.length)];if(!ns)throw Error('Unmapped '+r.path);return {path:r.path,entry_ids:ns.map(id),status:'mapped',notes:'Complete source/document read; exact algorithms and contracts are specified in the listed entries.'};});
const supplementary=copies.filter(c=>c.tracked).map(c=>({path:c.copy,lines:c.copy_lines-1,area:'blackhole',entry_ids:mapping[c.source.slice(prefix.length)].map(id),status:'mapped',notes:c.same_bytes?'Supplemental tracked runtime copy; byte-identical to fully read '+c.source+'. SHA-256 equality recorded in evidence/blackhole-copies.json.':'Supplemental tracked runtime copy fully read; formatting-only difference confirmed by normalized syntax or full git diff against '+c.source+'.'}));
const surfaces=[];
const add=(category,name,p,line,n,status='missing',rust_ref='')=>surfaces.push({category,name,ice_ref:prefix+p+':'+line,rust_ref,status,entry_id:id(n),candidate_ids:candidates.filter(c=>c.category===category&&c.name===name&&c.path===prefix+p).map(c=>c.candidate_id)});
add('slash-commands','/blackhole','index.ts',109,6);
for(const [name,line] of [['ICE_CODING_AGENT_DIR',40],['ICE_BLACKHOLE_COMPACTION',91],['ICE_BLACKHOLE_COMPACTION_ENGINE',95],['ICE_BLACKHOLE_MID_RUN_COMPACTION',103],['ICE_BLACKHOLE_COMPACT_AFTER_PERCENT',114]])add('environment',name,'src/core/unified-config.ts',line,name==='ICE_CODING_AGENT_DIR'?2:3);
for(const [name,line,n] of [['compaction',11,2],['compactionEngine',12,2],['midRunCompaction',13,2],['tailBehavior',14,2],['compactAfterPercent',15,2],['memory',16,2],['trimKeptToolResults',18,2],['refine',20,2],['refineModel',22,2]])add('settings',name,'src/core/unified-config.ts',line,n);
const indexText=fs.readFileSync(path.join(root,prefix,'index.ts'),'utf8').split('\n');
for(const name of ['compaction','compaction-engine','mid-run-compaction','threshold','tail-behavior','trim-kept-tool-results','refine','memory']){
 const line=indexText.findIndex(l=>l.includes('id: "'+name+'"'))+1;if(!line)throw Error('Missing settings ID '+name);
 add('settings','blackhole.'+name,'index.ts',line,5);
}
add('extension-api','registerSettings namespace blackhole','index.ts',75,5);
add('extension-api','Blackhole compactAfterTurn(mode)','src/om/compaction-trigger.ts',43,41,'partial','crates/ice-harness/src/agent_harness.rs:3867-3915');
add('paths','<agentDir>/ice-blackhole/ice-blackhole-config.json','src/core/unified-config.ts',39,2);
add('paths','<agentDir>/ice-cognee/compaction-recall.json','src/core/checkpoint-recall.ts',32,37);
for(const [name,p,line,n] of [['session_before_compact','src/hooks/before-compact.ts',25,39],['context','src/hooks/trim-kept-tail.ts',75,40],['agent_start','src/om/compaction-trigger.ts',49,41],['session_compact','src/om/compaction-trigger.ts',54,41],['turn_end','src/om/compaction-trigger.ts',58,41]])add('hooks',name,p,line,n);
for(const [name,p,line,n] of [['ice-goals-state','src/core/live-state.ts',11,32],['ice-safe-verify-state','src/core/live-state.ts',12,33],['bash_background_completed','src/core/live-state.ts',13,35],['web-search-results','src/core/live-state.ts',14,36],['ice-subagent-v2-child','src/core/subagents.ts',15,34],['ice-cognee-recall','src/core/tail.ts',30,31]])add('session-types',name,p,line,n);
add('session-types','compaction.details.engine=blackhole','src/hooks/before-compact.ts',99,39);
add('session-types','compaction.firstKeptEntryId=__blackhole_minimal_tail__','src/core/tail.ts',26,31);

// Resolve concrete declaration/call lines rather than retain guessed comment lines.
function exactLine(p,needle){const a=fs.readFileSync(path.join(root,prefix,p),'utf8').split('\n');const i=a.findIndex(l=>l.includes(needle));if(i<0)throw Error('No source occurrence: '+needle);return i+1;}
for(const s of surfaces){
 if(s.name==='ice-cognee-recall')s.ice_ref=prefix+'src/core/tail.ts:'+exactLine('src/core/tail.ts','"ice-cognee-recall"');
 if(s.name==='compaction.firstKeptEntryId=__blackhole_minimal_tail__')s.ice_ref=prefix+'src/core/tail.ts:'+exactLine('src/core/tail.ts','"__blackhole_minimal_tail__"');
 if(s.name==='Blackhole compactAfterTurn(mode)')s.ice_ref=prefix+'src/om/compaction-trigger.ts:'+exactLine('src/om/compaction-trigger.ts','ctx.compactAfterTurn(');
 if(s.category==='settings')s.notes=s.name.startsWith('blackhole.')?'UI namespace blackhole followed by exact item ID; not a persisted dotted JSON key.':'Exact top-level JSON key in ice-blackhole-config.json.';
}

const resolved=new Set(surfaces.flatMap(s=>s.candidate_ids));
const exclusions=candidates.filter(c=>!resolved.has(c.candidate_id)).map(c=>({candidate_id:c.candidate_id,reason:c.category==='paths'?'Filesystem target expression resolved to the exact config/checkpoint path in BLACK-'+(c.path.includes('checkpoint')?'037':'004')+'; not a separate user-addressable path. Source '+c.path+':'+c.line+'.':'Unexpected unresolved candidate: requires integrator review.'}));
if(exclusions.some(e=>e.reason.startsWith('Unexpected')))throw Error(JSON.stringify(exclusions));
const claims=[];
const claim=(a,b,text,verdict,finding,ns,refs)=>claims.push({first_pass_lines:[a,b],claim:text,verdict,finding,evidence:refs,entry_ids:ns.map(id)});
claim(386,387,'Optional Blackhole is missing as a plugin and deterministic engine.','confirmed','Rust core compaction exists but no Blackhole implementation/native plugin was found; TypeScript package cannot be loaded by the native ABI.',[1,39],['ice:'+prefix+'index.ts:153-159','rust:crates/ice-harness/src/compaction/compaction.rs:749-804']);
claim(389,390,'Compiler normalizes messages into deterministic sections.','confirmed','All normalized variants, source-index rules, sanitizer/filter and layout are specified, not only section names.',[7,8,9,26],['ice:'+prefix+'src/types.ts:13-36','ice:'+prefix+'src/core/summarize.ts:229-247']);
claim(390,391,'Compiler extracts goals/preferences/files/commits/outstanding/recent actions and builds a bounded brief.','corrected','Brief is bounded by line/content-word heuristics, not a global summary token budget. Findings and genuine user requests are also first-class sections. Exact limits and imperfect result/hash matching are specified.',[13,16,18,19,20,21,22,23,24,25,26],['ice:'+prefix+'src/core/build-sections.ts:138-159','ice:'+prefix+'src/core/format.ts:1-74']);
claim(392,392,'Live-state reconstructs Active Goal, Mode And Plan, Background Shells and MCP/external use.','confirmed','These are tolerant recorded-state projections, not runtime authority checks or live process polls.',[32,33,35,36],['ice:'+prefix+'src/core/live-state.ts:11-389']);
claim(393,393,'Subagent digest reads latest child records.','corrected','Latest valid data is per child ID, but the last12 cap follows first-seen Map insertion order, not most recently updated child. Malformed later records can leave prior state.',[34],['ice:'+prefix+'src/core/subagents.ts:51-111']);
claim(394,394,'tailBehavior supports ice-default/minimal with kept-tail and recall-noise handling.','confirmed','Current code default is minimal, contradicting provenance prose. Minimal uses a missing cut-ID sentinel and does not insert a resume user message.',[2,31],['ice:'+prefix+'src/core/unified-config.ts:25-34','ice:'+prefix+'src/core/tail.ts:29-87','ice:packages/coding-agent/src/core/session-manager.ts:427-462']);
claim(395,395,'Checkpoint recall appends a fresh session-matching bounded Relevant Memory section.','corrected','It deletes a matching valid checkpoint before rendering, mismatched host IDs leave file, expiry15min, line cap24, and the nominal4000-unit header/body budget excludes an additional45-unit notice. No top-level JSON size/schema cap or atomic claim.',[37],['ice:'+prefix+'src/core/checkpoint-recall.ts:1-114']);
claim(396,397,'Refinement is optional, model-routed, bounded and falls back to deterministic text.','corrected','Recent serialized input is capped24000 UTF-16 units and completion maxTokens1200; the deterministic summary is not itself bounded. Requested300 words is not enforced. Output is explicitly unverified and structural neutralization is narrow.',[38],['ice:'+prefix+'src/core/refine.ts:16-81','ice:'+prefix+'src/hooks/before-compact.ts:66-89']);
claim(398,398,'Before-compaction hook can override core and kept-tool outputs are trimmed.','confirmed','Exact gating, cancel/override result and context-only6000-unit text budget are specified.',[39,40],['ice:'+prefix+'src/hooks/before-compact.ts:24-111','ice:'+prefix+'src/hooks/trim-kept-tail.ts:12-83']);
claim(399,400,'Config at agentDir/ice-blackhole with enum/percent/refine/memory settings.','confirmed','Exact default/precedence/write permission behavior specified, including unused memory flag and optional refineModel.',[2,3,4],['ice:'+prefix+'src/core/unified-config.ts:5-157']);
claim(401,403,'registerSettings and /blackhole provide status/percent/resume/pause/off.','confirmed','Eight UI IDs and exact parser/preset/save behavior specified.',[5,6],['ice:'+prefix+'index.ts:12-151']);
claim(410,412,'Mid-run compaction is missing in Rust; compaction exists mainly as manual flow.','corrected','Rust installs an AfterToolResults closure, checks threshold, compacts/persists and replaces context before the next model step. Blackhole-specific percent settings, pause/resume request protocol and suppression state remain absent.',[41],['rust:crates/ice-harness/src/agent_harness.rs:3867-3915','rust:crates/ice-harness/src/agent_harness.rs:3957-3969','ice:'+prefix+'src/om/compaction-trigger.ts:21-68']);
const tests=[['src/core/checkpoint-recall.test.ts',188],['src/core/live-state.test.ts',212],['src/core/sections.test.ts',205],['src/core/subagents.test.ts',92],['src/core/tail.test.ts',83],['src/core/unified-config.test.ts',85],['src/hooks/trim-kept-tail.test.ts',85]];
const evidence={area:'blackhole',complete:true,entries:spec.entries.map(({id,title,status,priority,size_min,size_max,dependencies})=>({id,title,status,priority,size_min,size_max,dependencies})),ledger,supplemental_ledger:supplementary,surfaces,candidate_exclusions:exclusions,claims,read_evidence:[...original.map(r=>({path:'ice:'+r.path,ranges:[[1,r.lines+1]],notes:'Complete source/document read.'})),...tests.map(([p,n])=>({path:'ice:'+prefix+p,ranges:[[1,n]],notes:'Existing test read, not executed.'})),{path:'ice:packages/coding-agent/test/blackhole-review-repairs.test.ts',ranges:[[1,164]]},{path:'ice:packages/coding-agent/test/suite/blackhole-compaction.test.ts',ranges:[[1,578]]},...copies.filter(c=>c.same_bytes===false).map(c=>({path:'ice:'+c.copy,ranges:[[1,c.copy_lines]],notes:'Complete read and formatting-equivalence verification.'}))],rust_only:[],open_questions:[{id:'Q-BLACK-01',question:'Should the Rust port preserve the current optional/unused memory setting and minimal-tail default, or revise the stale provenance/UI contract?',entries:[id(1),id(2),id(5)]},{id:'Q-BLACK-02',question:'Should checkpoint consumption be hardened to atomic claim, validated top-level JSON and a true whole-section budget? Current TS is one-shot by deletion but not race-safe, and the notice sits outside the4000-unit budget.',entries:[id(37)]},{id:'Q-BLACK-03',question:'Should old model-inferred State Of Work be explicitly stripped during re-compaction and should whitespace-prefixed forged headers be neutralized? Current source does neither comprehensively.',entries:[id(30),id(38)]},{id:'Q-BLACK-04',question:'For cross-language fidelity, which Unicode word segmentation and UTF-16 clipping behavior is authoritative when Rust Unicode/ICU output differs?',entries:[id(10),id(13),id(26)]}],unverified:[],dependency_requests:[{entry_id:id(5),area:'extensions',capability:'namespaced registerSettings descriptors/change callbacks'},{entry_id:id(26),area:'tui',capability:'wrapTextWithAnsi display-width and continuation behavior'},{entry_id:id(31),area:'session',capability:'compaction cut-ID/retained-tail override and durable context projection'},{entry_id:id(34),area:'subagents',capability:'ice-subagent-v2-child durable records and artifact schemas'},{entry_id:id(35),area:'tools',capability:'background Bash IDs, polling result shapes, completion custom messages'},{entry_id:id(37),area:'cognee',capability:'session-scoped compaction-recall.json producer'},{entry_id:id(38),area:'ai',capability:'complete/model registry/cancellation/no-cache session isolation'},{entry_id:id(39),area:'extensions',capability:'session_before_compact preparation and override result'},{entry_id:id(41),area:'session',capability:'compactAfterTurn pause/resume scheduling and session_compact acknowledgement'}]};
fs.writeFileSync(path.join(base,'evidence/blackhole.json'),JSON.stringify(evidence,null,2)+'\n');
console.log(JSON.stringify({entries:spec.entries.length,ledger_rows:ledger.length,supplemental_rows:supplementary.length,surface_rows:surfaces.length,candidates_resolved:resolved.size,exclusions:exclusions.length,claims:claims.length,corrected:claims.filter(c=>c.verdict==='corrected').length},null,2));
