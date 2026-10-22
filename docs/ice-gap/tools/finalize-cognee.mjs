import { auditHelperBase } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
assertAuditOpen();
// Finalize only directly inspected Cognee evidence. No source execution, network or agents.
import fs from 'node:fs';
import path from 'node:path';
const base=auditHelperBase;
const tsroot='/home/mewtwo/Zks/ice';
const ca='packages/coding-agent/';
const main=ca+'src/ice-cognee.ts', client=ca+'src/ice-cognee-client.ts', env=ca+'src/ice-cognee-env.ts', obs=ca+'src/ice-cognee-observer.ts';
const load=p=>JSON.parse(fs.readFileSync(path.join(base,p),'utf8'));
const spec=load('evidence/spec-cognee.json');
if(spec.entries.length!==68)throw Error('Expected 68 complete Cognee entries');
const id=n=>'COG-'+String(n).padStart(3,'0');
const get=n=>spec.entries.find(e=>e.id===id(n));
function quote(p,a,b){return '```ts\n'+fs.readFileSync(path.join(tsroot,p),'utf8').split('\n').slice(a-1,b).join('\n')+'\n```\nSource: ice:'+p+':'+a+'-'+b+'.';}
// Corrections caught by final citation review; preserve verbatim source blocks.
get(21).interfaces=quote(client,572,734)+'\n'+quote(client,788,836)+'\nNone: additional CLI flags, slash commands, keybindings, model-facing tools or agent RPC commands.';
get(21).source=get(21).source.replace('ice-cognee-client.ts:572-741','ice-cognee-client.ts:572-734').replace('ice-cognee-client.ts:802-836','ice-cognee-client.ts:788-836');
const q27=get(27);q27.mechanism=q27.mechanism.slice(0,q27.mechanism.indexOf('```ts'))+quote(client,724,800);q27.source=q27.source.replace('ice-cognee-client.ts:742-800','ice-cognee-client.ts:724-800');
get(27).interfaces=get(27).interfaces.replace('MAX_NODE_TEXT','MAX_GRAPH_NODE_TEXT');
get(39).interfaces=get(39).interfaces.replace('DREAM_IMPROVE_LEASE_MS','DREAM_CLAIM_LEASE_MS');
get(60).interfaces=get(60).interfaces.replace('MAX_COGNEE_OBSERVATIONS','MAX_EVENTS');
get(58).mechanism=get(58).mechanism.replace('getCogneeSkillsDir resolves the directory beside the shipped ice-cognee module through the compatibility path helper.','getCogneeSkillsDir is exported by config.ts. For a Bun binary it returns <packageDir>/ice-cognee-skills; otherwise it chooses src when <packageDir>/src exists, else dist, then appends ice-cognee-skills. PackageDir obeys ICE_PACKAGE_DIR before runtime discovery.');
get(58).interfaces=quote(main,2037,2040)+'\n'+quote(ca+'src/config.ts',463,476)+'\nBundled documents are reproduced in the appendix with original paths and complete text. No additional CLI flags, keybindings or agent RPC commands.';
get(58).source=get(58).source.replace('ice:'+ca+'src/core/legacy-compat/cognee.ts:1-9','ice:'+ca+'src/config.ts:369-390; ice:'+ca+'src/config.ts:463-476');
get(40).mechanism=get(40).mechanism.replace('If the claim covers current revision, unlink the record;','If the claim covers current revision, attempt to unlink the record and swallow unlink errors;');
get(43).mechanism+='\n\nThe compatibility environment normalizer currently performs only a shallow copy; it does not map old variable names. A rejected tracked promise has a finally-derived promise whose rejection is not explicitly caught by trackBackground itself; current callers generally catch their operations, but tracking alone is not a rejection-isolation boundary. Source: ice:'+ca+'src/core/legacy-compat/env.ts:17-19 and ice:'+main+':1178-1181.';
get(43).source+=' Supplemental helper: ice:'+ca+'src/core/legacy-compat/env.ts:1-32.';
get(43).rust+=' The first-pass reference to an ice-memory companion implementation could not be verified inside the local crates/packages trees; no such package directory is present. Its external behavior remains unverified because network calls are prohibited. This does not change the source-backed absence of the Cognee integration in these trees.';
// Fix known prose-only numeric spacing without changing identifiers or fenced source.
const spaced=[[/\b(first|last|limit|default|maximum|minimum|take|max|cap|threshold|retries|stale|timeout)(\d)/g,'$1 $2'],[/\b(at|than|to|after|before|within|over|under|below|exceeds|above|every|of|with)(\d)/g,'$1 $2'],[/(\d)(minutes|minute|seconds|second|days|day|files|tokens|ms|KiB|bytes)\b/g,'$1 $2']];
for(const e of spec.entries)for(const field of ['what','mechanism','interfaces','source','port','priority_reason'])if(e[field])e[field]=e[field].split(/(```[\s\S]*?```)/g).map((s,i)=>{if(i%2)return s;for(const [re,replacement]of spaced)s=s.replace(re,replacement);return s;}).join('');
fs.writeFileSync(path.join(base,'evidence/spec-cognee.json'),JSON.stringify(spec,null,2)+'\n');
const candidates=load('evidence/surface-candidates.json').filter(c=>c.area==='cognee');
const assigned=load('evidence/tracked-files.json').filter(r=>r.area==='cognee');
const numbers=(a,b)=>Array.from({length:b-a+1},(_,i)=>a+i);
const ledger=assigned.map(r=>{let ns;if(r.path===main)ns=[1,3,4,5,6,7,8,...numbers(31,58),...numbers(61,68)];else if(r.path===client)ns=numbers(9,30);else if(r.path===env)ns=[2];else if(r.path===obs)ns=[59,60];else if(r.path.includes('/ice-cognee-skills/'))ns=[58];else if(r.path===ca+'docs/ice-cognee.md')ns=[1,2,3,4,35,36,43,46,47,48,49,50,51,52,53,54,55,56,57,58,59,62,63,64,65,66,67,68];else throw Error('Unknown ledger row '+r.path);return {path:r.path,entry_ids:ns.map(id),status:'mapped',notes:'Complete source/document read; exact configuration, transport, persistence, lifecycle and UI mechanisms are specified in the listed entries. Tests were read, not executed.'};});
const surfaces=[];
const candidateIds=ns=>ns.map(n=>'C'+String(n).padStart(6,'0'));
function add(category,name,p,line,n,ns=[],notes=''){surfaces.push({category,name,ice_ref:p+':'+line,rust_ref:'',status:'missing',entry_id:id(n),candidate_ids:candidateIds(ns),notes});}
// Typed/default declarations collapse to actual supported keys, never a fake nested settings namespace.
const settings={};
for(const c of candidates.filter(c=>c.category==='settings')){const key=c.name.split('.').at(-1);(settings[key]??=[]).push(c);}
for(const [key,cs]of Object.entries(settings)){const c=cs.find(c=>c.kind==='default-setting')??cs[0];add('settings',key==='apiKey'?'CogneeClientConfig.apiKey':key==='maxResponseChars'?'CogneeClientConfig.maxResponseChars':'ice-cognee/config.json:'+key,c.path,c.line,key==='apiKey'?3:1,[],key==='apiKey'?'Embedding client option; not accepted or persisted by the config-file parser.':key==='maxResponseChars'?'Derived/embedding response limit, not a persisted editable JSON key.':'Exact top-level JSON key in the Cognee-specific config file.');surfaces.at(-1).candidate_ids=cs.map(c=>c.candidate_id);}
add('environment','COGNEE_API_KEY',main,897,3,[828]);
for(const [name,line]of [['ICE_COGNEE_ENABLED',337],['ICE_COGNEE_RECALL',339],['ICE_COGNEE_REMEMBER',340],['ICE_COGNEE_BASE_URL',351],['COGNEE_BASE_URL',351],['ICE_COGNEE_DATASET',352],['ICE_COGNEE_CAPTURE',354],['ICE_COGNEE_CAPTURE_TOOLS',357],['ICE_COGNEE_IMPROVE',360],['ICE_COGNEE_COMPACTION_SUMMARY',361],['ICE_COGNEE_CHECKPOINT_RECALL',370]])add('environment',name,main,line,1);
add('environment','COGNEE_LOCAL_API_URL',env,64,2);
for(const [name,line]of [['COGNEE_PLUGIN_DATASET',14],['COGNEE_DATASET',15],['COGNEE_SESSION_ID',16]])add('environment',name,env,line,2,[],'Accepted into merged environment only; not used as runtime dataset/session identity by the Cognee resolver.');
add('environment','COGNEE_* / ICE_COGNEE_* env-file acceptance',env,41,2);
// Canonical paths replace local variable-expression candidates after reading their callers.
add('paths','~/.cognee/.env',env,54,2,[777]);
add('paths','<storageDir>/observations.jsonl',obs,55,59,[778,779,780]);
add('paths','<storageDir>/observations.jsonl.1',obs,100,60,[781]);
add('paths','<agentDir>/ice-blackhole/ice-blackhole-config.json',main,141,51,[813]);
add('paths','<target>.<uuid>.tmp',main,467,42,[814,815,816]);
add('paths','<storageDir>/pending/',main,1135,33,[817,818,819]);
add('paths','<storageDir>/pending/<operationId>.json',main,503,34,[820,821]);
add('paths','<storageDir>/dream/<sessionId>.json',main,631,37,[822]);
add('paths','<storageDir>/dream/<sessionId>.<sha256(dataset)>.json',main,646,37,[825,826]);
add('paths','<dream-record>.lock',main,653,38,[823]);
add('paths','<dream-record> acknowledgement removal',main,725,40,[824]);
add('paths','<storageDir>/config.json',main,1134,6,[827,830]);
add('paths','<agentDir>/ice-cognee/api_key.json',main,903,3,[829]);
add('paths','~/.cognee-plugin/api_key.json',main,902,3);
add('paths','<storageDir>/warmup/<timestamp>-<uuid-prefix>.json',main,1270,36,[832,833,834,835,836,837]);
add('paths','<storageDir>/compaction-recall.json',main,2289,51,[846,847,848,849]);
add('paths','<packageDir>/{src|dist}/ice-cognee-skills/',ca+'src/config.ts',469,58);
add('session-types','ice-cognee',main,1044,35,[831]);
add('session-types','compaction.details.engine=ice-cognee',main,2431,52);
add('tools','cognee_search',main,1780,55,[838]);
add('slash-commands','/cognee',main,1825,56,[839]);
for(const [name,line,n]of [['status',1841,67],['doctor',1851,67],['watch',1831,67],['compact auto|defer|own',1877,66],['on|off',1886,66],['recall on|off',1890,66],['capture on|off',1894,66],['tools on|off',1899,66],['improve on|off',1904,66],['improve [now]',1909,65],['remember on|off',1930,66],['search <query>',1934,63],['remember [node_set] <text>',1938,64],['flush [pending|uncertain]',1945,68]])add('slash-commands','/cognee '+name,main,line,n);
for(const [name,line,n,ci]of [['resources_discover',2038,58,840],['session_start',2043,62,841],['before_agent_start',2106,46,842],['tool_result',2194,47,843],['agent_end',2244,48,844],['agent_settled',2276,49,845],['session_before_compact',2376,52,850],['session_compact',2441,53,851],['session_shutdown',2506,54,852]])add('hooks',name,main,line,n,[ci],'This row is the Cognee handler at this registration site, not a claim that the generic host event is absent from Rust.');
for(const [name,line]of [['enabled',1964],['autoRecall',1971],['autoRemember',1978],['compactionSummaryMode',1985],['checkpointRecall',1993],['captureSession',2000],['captureTools',2007],['autoImprove',2014]])add('settings','cognee.'+name+' (settings UI)',main,line,57,[],'Namespace plus UI item ID, not a nested core settings.json field.');
add('extension-api','registerSettings namespace cognee',main,1961,57);
add('extension-api','Cognee observation subscription',obs,70,60);
add('tui','Cognee status/activity line',main,1359,61);
for(const [name,line]of [['session',12],['recall',13],['remember_entry',14],['remember',15],['trace',16],['improve',17],['queue',18]])add('observability','Cognee operation:'+name,obs,line,59);
for(const phase of ['started','queued','succeeded','failed'])add('observability','Cognee phase:'+phase,obs,19,59);
add('providers-auth','Cognee HTTP x-api-key authentication',client,376,9);
// Exact source expressions used in a path row are checked here, not guessed from a variable name.
function exactLine(p,needle){const i=fs.readFileSync(path.join(tsroot,p),'utf8').split('\n').findIndex(l=>l.includes(needle));if(i<0)throw Error('Missing path discriminator '+needle);return i+1;}
const correct=[['<storageDir>/dream/<sessionId>.json',main,'`${sessionId}.json`'],['<storageDir>/dream/<sessionId>.<sha256(dataset)>.json',main,'function scopedDreamPath'],['<dream-record>.lock',main,'lockfile.lock('],['<agentDir>/ice-cognee/api_key.json',main,'compatibleCogneeStorageDir(getAgentDir()), "api_key.json"'],['~/.cognee-plugin/api_key.json',main,'".cognee-plugin", "api_key.json"'],['<storageDir>/compaction-recall.json',main,'"compaction-recall.json"'],['compaction.details.engine=ice-cognee',main,'engine: "ice-cognee"']];
for(const [name,p,needle]of correct){const s=surfaces.find(s=>s.name===name);s.ice_ref=p+':'+exactLine(p,needle);}
const used=new Set(surfaces.flatMap(s=>s.candidate_ids));
const unresolved=candidates.filter(c=>!used.has(c.candidate_id));if(unresolved.length)throw Error('Unresolved candidates '+unresolved.map(c=>c.candidate_id));
const claims=[];
function claim(a,b,text,verdict,finding,ns,evidence){claims.push({first_pass_lines:[a,b],claim:text,verdict,finding,evidence,entry_ids:ns.map(id)});}
claim(352,355,'Cognee integration is absent; an ice-memory plugin is unrelated.','corrected','The local Rust crates/packages have no Cognee client/lifecycle integration. A companion ice-memory implementation is not present in this checkout, so its external behavior is unverified rather than asserted equivalent or unrelated. TS source sizes and bundled skill paths match the inspected files.',[43,58],['ice:'+main+':1132-1172','rust:crates/ice-extensions/src/actions.rs:381-499','rust:crates/ice-harness/src/compaction/compaction.rs:749-804']);
claim(358,360,'Prompt hook adds scoped recall, skips under four words and deduplicates.','confirmed','Signal words contain a Unicode letter/number; successful exact session+raw-prompt requests suppress repeats for 15000 ms. Failures remain retryable and short/skipped prompts can still consume a last-compaction injection.',[46],['ice:'+main+':1113-1116','ice:'+main+':2106-2192']);
claim(361,362,'Tool results capture allowlisted tools with a per-turn limit.','confirmed','Exact allowlist and lower-case acceptance, 24 attempted traces per prompt, per-parameter caps and textual result capture are specified. Failed capture still consumes the slot.',[47],['ice:'+main+':22-46','ice:'+main+':2194-2242']);
claim(363,364,'Agent end remembers QA and idle improves/dreams with cooldown.','corrected','QA uses one pending prompt and the last assistant text, regardless of stop reason. Current-session improve and old-session dream are distinct, revision-fenced flows. HTTP acceptance is not server consolidation completion; warmup retry is a third flow.',[36,38,39,40,48,49,50],['ice:'+main+':1319-1357','ice:'+main+':1432-1549','ice:'+main+':2244-2286']);
claim(365,368,'Compaction can be owned/contributed and checkpoint recall is handed to Blackhole.','corrected','Default auto/defer never owns the summary. Explicit own compiles a local anchor and visible text without a Cognee summary request. Checkpoint recall is separately opt-in, config-detected Blackhole-only, and binds the handoff to the original host session ID.',[51,52,53],['ice:'+main+':139-195','ice:'+main+':2288-2504']);
claim(369,369,'Shutdown runs final improve.','confirmed','Tracked captures/drains settle before final current-session improve and optional unregister. No global shutdown deadline or server completion poll is provided.',[54],['ice:'+main+':2506-2529']);
claim(370,370,'Resource discovery contributes Cognee skills.','confirmed','The three bundled skill documents are always returned, even when memory is disabled; runtime-specific asset resolution is in config.ts, not the storage-compatibility helper.',[58],['ice:'+main+':2037-2040','ice:'+ca+'src/config.ts:463-476']);
claim(371,372,'$project derives ice-<root-name>-<hash>.','corrected','Root is a lexical resolved path found by at most 24 .git-existence checks, not realpath or Git canonical root. Basename is sanitized/clipped to 40 and SHA-256 prefix is eight hex characters. Default dataset is literal ice, not $project.',[1,4],['ice:'+main+':305-331','ice:'+main+':111-133']);
claim(373,375,'Local HTTP client with a circuit breaker and env/cache credential sources; shared .env is nonsecret-only.','corrected','The default endpoint is loopback but any HTTP/S URL is accepted. Two extension-owned breakers gate selected automatic paths, not all client requests. Credential order includes explicit options, process, ICE cache, shared-plugin cache and shared .env. The parser explicitly accepts COGNEE_API_KEY, so the nonsecret-only claim is false.',[1,2,3,9,31,45],['ice:'+main+':280-290','ice:'+main+':868-907','ice:'+env+':10-69','ice:'+main+':933-952']);
claim(376,377,'cognee_search, slash commands and a settings section are available.','confirmed','Full tool schema, 14 command forms/branch precedence and eight settings descriptors are specified; manual improve and remember have different gating/replay rules from automatic flows.',[55,56,57,63,64,65,66,67,68],['ice:'+main+':1780-2035']);
claim(378,379,'Every request is appended redacted to a size-capped observations log.','corrected','Only instrumented call sites emit observations. The observer redacts preview only; some error/metadata/base URL displays remain unsanitized. Rotation occurs after append at more than 2 MiB, keeps one .1 generation and is serialized only within a process. Reads return at most 500 valid-ish recent lines but read full files first.',[45,59,60],['ice:'+obs+':62-128','ice:'+main+':1211-1253','ice:'+main+':1720-1823']);
claim(380,380,'Ten ICE_COGNEE_* environment overrides exist.','confirmed','All ten are resolved with strict types and exact names. COGNEE_BASE_URL/LOCAL_API_URL and API-key sources are additional supported inputs; numeric settings do not have environment overrides.',[1,2,3],['ice:'+main+':333-398','ice:'+env+':52-69']);
const dependency_requests=[
 [1,'settings','canonical agent directory and no-op legacy environment normalization'],[3,'settings','credential directory location and source-safe diagnostic handling'],[7,'misc','shared credential redaction regexes and UTF-16 compatibility'],[43,'extensions','hidden before-user extension initialization, session identity and lifecycle delivery'],[44,'extensions','tool-result argument/result payload and session-safe lifecycle contexts'],[46,'extensions','before_agent_start system-prompt replacement and cancellation'],[51,'session','compaction preparation/reason/signal and host session identity'],[52,'extensions','session_before_compact override protocol'],[53,'session','post-compaction event details and ordered custom-entry persistence'],[55,'extensions','tool registration with schema/cancellation/details and active-tool state'],[57,'extensions','namespaced settings descriptors and change callbacks'],[58,'settings','bundled skill discovery through the resource loader'],[59,'observability','live Cognee observation projection into the run recorder'],[61,'tui','status slots and safe lifecycle of retired UI contexts']
].map(([n,area,capability])=>({entry_id:id(n),area,capability}));
const read_evidence=[...assigned.map(r=>({path:'ice:'+r.path,ranges:[[1,r.lines+1]],notes:'Full assigned source/document read.'})),...[
 ['ice-cognee.test.ts',1770],['ice-cognee-transport.test.ts',447],['ice-cognee-browse.test.ts',219],['ice-cognee-observer.test.ts',66],['observe-cognee.test.ts',168]
].map(([p,n])=>({path:'ice:'+ca+'test/'+p,ranges:[[1,n]],notes:'Existing test read, not executed.'})),{path:'ice:'+ca+'src/utils/redact.ts',ranges:[[1,9]]},{path:'ice:'+ca+'src/core/legacy-compat/cognee.ts',ranges:[[1,10]]},{path:'ice:'+ca+'src/core/legacy-compat/env.ts',ranges:[[1,32]]},{path:'ice:'+ca+'src/config.ts',ranges:[[1,605]]},{path:'rust:crates/ice-extensions/src/actions.rs',ranges:[[381,499],[881,923]]},{path:'rust:crates/ice-agent/src/hooks.rs',ranges:[[99,130]]},{path:'rust:crates/ice-harness/src/compaction/compaction.rs',ranges:[[749,804]]},{path:'rust:crates/ice-plugin-sdk/src/events.rs',ranges:[[1,224]]}];
const evidence={area:'cognee',complete:true,entries:spec.entries.map(({id,title,status,priority,size_min,size_max,dependencies})=>({id,title,status,priority,size_min,size_max,dependencies})),ledger,supplemental_ledger:[],surfaces,candidate_exclusions:[],claims,read_evidence,rust_only:[],dependency_requests,unverified:[{entry_id:id(43),finding:'External ice-memory plugin implementation is not in the local checkout and network access is prohibited. No parity claim is made about that external package.'}],open_questions:[
 {id:'Q-COG-01',question:'Should Rust retain enabled capture/recall by default and unrestricted HTTP/S endpoint selection, or add an explicit consent/trust boundary before sending prompt/tool data?',entries:[id(1),id(7),id(46),id(47)]},
 {id:'Q-COG-02',question:'Should ambiguous manual remembers and warmup writes be quarantined rather than automatically replayed, and should pending records gain path validation and cross-process claims?',entries:[id(32),id(34),id(35),id(36),id(64)]},
 {id:'Q-COG-03',question:'Should late capture/registration/observation completions be fenced to their original session, and should all state reset when one extension instance changes session?',entries:[id(43),id(44),id(59),id(62)]},
 {id:'Q-COG-04',question:'Should health and shutdown receive hard total deadlines, all observed fields be redacted, and successful improve mean a confirmed server job completion rather than request acceptance?',entries:[id(10),id(17),id(45),id(49),id(54),id(59)]},
 {id:'Q-COG-05',question:'Should manual improve obey enabled/claim/in-flight policy, settings writes be awaited/serialized, and the displayed settings values refresh from loaded configuration?',entries:[id(57),id(65),id(66)]}
]};
fs.writeFileSync(path.join(base,'evidence/cognee.json'),JSON.stringify(evidence,null,2)+'\n');
console.log(JSON.stringify({entries:spec.entries.length,ledger_rows:ledger.length,surface_rows:surfaces.length,resolved_candidates:used.size,claims:claims.length,unverified:evidence.unverified.length},null,2));
