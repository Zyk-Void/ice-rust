// Central prose spacing pass. Source files and code spans are never edited.
import fs from 'node:fs'; import path from 'node:path'; import crypto from 'node:crypto'; import {fileURLToPath} from 'node:url'; import {execFileSync} from 'node:child_process';
const B=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..'), E=path.join(B,'evidence');
const areas=['ai','agent','tools','session','settings','extensions','rpc-remote','tui','subagents','safe-verify','observability','cognee','blackhole','providers-auth','cli-packaging','misc'];
const read=n=>JSON.parse(fs.readFileSync(path.join(E,n),'utf8')), sha=s=>crypto.createHash('sha256').update(s).digest('hex');

function runLegacy(){
const mode=process.argv[2]??'preview';if(!['preview','apply'].includes(mode))throw Error('preview or apply');
if(read('progress.json').complete)throw Error('Completed audit protected from historical mutation');
const fields=['what','mechanism','interfaces','rust','port','priority_reason','dependency_text'];
const words='all|main|a|an|the|and|or|to|from|after|before|at|by|with|than|under|over|of|in|is|has|for|through|timeout|default|defaults|maximum|minimum|limit|within|exactly|including|lines|line|exit|above|below|between|every|first|last|least|most|only|otherwise|channel|version|width|height|depth|rows|times|uses|bounds|remaining|requires|allows|accepts|exceeds|reaches|caps|capped|waits|one|two|up|down|size|range|mode|key|keys|value|values|length|count|counts|initial|start|port|return|returns|step|wait|plus|minus|takes|keeps|ceilings|ceiling|cost|type|stop|seconds|window|budget|defaulting|using|around|roughly';
const glue=new RegExp('^(?:'+words+')(?:\\d+(?:ms|s|seconds?|minutes?|hours?|bytes?|bits?|chars?|symbols|states|entries|MiB|KiB|px|fps|hex)?)$','i');
const wordNum=new RegExp('(?<![#/._-])\\b('+words+')(?=\\d+(?:[\\s,;:.)-]|(?:ms|s|seconds?|minutes?|hours?|bytes?|bits?|chars?|symbols|states|entries|MiB|KiB|px|fps|hex)\\b|\\.\\.))','gi');
const cite=/\b(?:ice|rust):[A-Za-z0-9_./@+\-]+:\d+(?:[-–]\d+)?/g;
const protect=/(\x60{3}[\s\S]*?\x60{3}|\x60[^\x60]*\x60|https?:\/\/[^\s\x60<>]+|(?:ice|rust):[A-Za-z0-9_./@+\-]+:\d+(?:[-–]\d+)?|(?:~?\/|(?:crates|packages|scripts|docs)\/)[A-Za-z0-9_./@+\-]+(?::\d+(?:[-–]\d+)?)?|\b[A-Za-z0-9_.-]+\.(?:rs|ts|tsx|mjs|js|json|toml|md|csv|py)(?::\d+(?:[-–]\d+)?)?|\b0x[0-9a-f]+\b|\b[0-9a-f]{7,64}\b|\bv?\d+\.\d+\.\d+(?:[-+][A-Za-z0-9.-]+)?\b|\b\d+\.\.\d+\b|"(?:\\.|[^"\\\n])*"|'(?:\\.|[^'\\\n])*'|(?<=^|[=(:;,\s])\/(?:\\.|[^\/\n])+\/[dgimsuvy]*)/gi;
const exports=new Set(Object.values(read('qa-unnamed-exports-2026-10-20.json').symbols).flat().map(r=>r.symbol));
const ids=text=>[...text.matchAll(/\b[A-Za-z_][A-Za-z0-9_]*\b/g)].map(m=>m[0]).filter(t=>!glue.test(t)&&!['KiB','MiB','GiB','ms','fps','px'].includes(t)&&(exports.has(t)||/_|[a-z][A-Z]|\d/.test(t)));
const sorted=s=>[...new Set(s)].sort(), same=(a,b)=>JSON.stringify(a)===JSON.stringify(b);
const report={schema_version:1,captured_at:new Date().toISOString(),owner:'/root',mode,scope:fields,method:'Only a fixed set of prose words glued to numeric values is split; code-like identifiers are excluded. Quotes, code, paths, citations, regex literals, ranges, hashes, versions and URLs are protected. After-comma spacing changes delimiters only. The citation and identifier inventories and every protected span are compared byte-for-byte.',inputs:[],changes:[],protected_span_mismatches:[],identifier_diffs:[],citation_diffs:[],behavioral_openings:[]};
const allBefore={citations:[],identifiers:[]},allAfter={citations:[],identifiers:[]},out=[];
function edit(part,id,field){
 let s=part.replace(wordNum,(m,word,offset)=>{if(exports.has(m+part.slice(offset+m.length).match(/^\d+\w*/)?.[0]))return m;const after=word+' ';report.changes.push({id,field,kind:'word-number',before:m,after,context:part.slice(Math.max(0,offset-35),offset+m.length+45)});return after;});
 s=s.replace(/\b(\d+(?:\.\d+)?)(ms|s|seconds?|minutes?|hours?|bytes?|bits?|chars?|symbols|states|entries|MiB|KiB|px|fps|hex)(?=\b)/g,(m,n,u,offset)=>{const after=n+' '+u;report.changes.push({id,field,kind:'number-unit',before:m,after,context:s.slice(Math.max(0,offset-35),offset+m.length+40)});return after;});
 s=s.replace(/,(?=[A-Za-z])/g,(m,offset)=>{report.changes.push({id,field,kind:'comma-space',before:',',after:', ',context:s.slice(Math.max(0,offset-35),offset+45)});return ', ';});
 return s;
}
for(const area of areas){
 const name='spec-'+area+'.json',raw=fs.readFileSync(path.join(E,name),'utf8'),s=JSON.parse(raw);report.inputs.push({name,sha256:sha(raw)});
 for(const e of s.entries){
  const opening=e.mechanism?.trim().split(/\n\s*\n/)[0]??'';
  report.behavioral_openings.push({id:e.id,first_sentence:opening.split(/(?<=[.!?])\s/)[0],valid:!!opening&&!/^(?:\x60{3}|#{1,6}\s|[-*]\s|ice:|rust:|Sources?:|See |Implementation (?:is|in)|This entry (?:describes|covers|documents))/i.test(opening)});
  for(const field of fields){
   const before=e[field];if(typeof before!=='string')continue;
   const parts=before.split(protect),after=parts.map((p,i)=>i%2?p:edit(p,e.id,field)).join('');
   const oldProtected=parts.filter((_,i)=>i%2),newProtected=after.split(protect).filter((_,i)=>i%2);
   let protectedCursor=0; const protectedRetained=oldProtected.every(value=>{const at=after.indexOf(value,protectedCursor); if(at<0)return false;protectedCursor=at+value.length;return true;});
   if(!protectedRetained)report.protected_span_mismatches.push({id:e.id,field,before:oldProtected.map(sha),after:newProtected.map(sha)});
   const bc=sorted([...before.matchAll(cite)].map(m=>m[0])),ac=sorted([...after.matchAll(cite)].map(m=>m[0]));
   const bi=sorted(ids(before)),ai=sorted(ids(after));
   const diff=(a,b)=>({removed:a.filter(v=>!b.includes(v)),added:b.filter(v=>!a.includes(v))});
   if(!same(bc,ac))report.citation_diffs.push({id:e.id,field,...diff(bc,ac)});
   if(!same(bi,ai))report.identifier_diffs.push({id:e.id,field,...diff(bi,ai)});
   allBefore.citations.push(...bc);allAfter.citations.push(...ac);allBefore.identifiers.push(...bi);allAfter.identifiers.push(...ai);
   e[field]=after;
  }
 }
 out.push({area,name,s});
}
report.summary={changes:report.changes.length,changed_entries:new Set(report.changes.map(c=>c.id)).size,by_kind:Object.fromEntries([...new Set(report.changes.map(c=>c.kind))].map(k=>[k,report.changes.filter(c=>c.kind===k).length])),protected_span_mismatches:report.protected_span_mismatches.length,identifier_diffs:report.identifier_diffs.length,citation_diffs:report.citation_diffs.length,behavioral_openings:report.behavioral_openings.length,invalid_openings:report.behavioral_openings.filter(x=>!x.valid).length};
report.extracted={citations:{before:sorted(allBefore.citations),after:sorted(allAfter.citations)},identifiers:{before:sorted(allBefore.identifiers),after:sorted(allAfter.identifiers)}};
const dry=path.join(E,'spacing-dry-run.json');
if(mode==='preview'){fs.writeFileSync(dry,JSON.stringify(report,null,2)+'\n');}
else{
 const prior=read('spacing-dry-run.json'),approval=read('spacing-review.json');
 if(approval.dry_run_sha256!==sha(fs.readFileSync(dry))||approval.approved!==true||(approval.reviewed_change_indexes?.length??0)<50)throw Error('At least 50 dry-run changes need review approval');
 if(!same(prior.inputs,report.inputs))throw Error('Specs changed after reviewed dry run');
 if(report.protected_span_mismatches.length||report.identifier_diffs.length||report.citation_diffs.length)throw Error('Protected evidence or identifiers changed');
 for(const o of out){
  fs.writeFileSync(path.join(E,o.name),JSON.stringify(o.s,null,2)+'\n');
  for(const n of[o.area+'.json','proposal-'+o.area+'.json'])if(fs.existsSync(path.join(E,n))){const m=read(n);for(const me of m.entries){const entry=o.s.entries.find(e=>e.id===me.id);for(const f of fields)if(Object.hasOwn(me,f))me[f]=entry[f];}fs.writeFileSync(path.join(E,n),JSON.stringify(m,null,2)+'\n');}
  const renderer=fs.existsSync(path.join(B,'tools/render-area.mjs'))?path.join(B,'tools/render-area.mjs'):path.join(B,'render-area.mjs');execFileSync(process.execPath,[renderer,o.area],{stdio:'pipe'});
 }
 report.approval=approval;report.citations_byte_identical=true;report.identifiers_byte_identical=true;report.protected_spans_byte_identical=true;
 fs.writeFileSync(path.join(E,'spacing-validation.json'),JSON.stringify(report,null,2)+'\n');
}
console.log(JSON.stringify({mode,summary:report.summary,protected_span_mismatches:report.protected_span_mismatches.slice(0,3),identifier_diffs:report.identifier_diffs.slice(0,8),citation_diffs:report.citation_diffs,invalid_openings:report.behavioral_openings.filter(x=>!x.valid),sample:report.changes.slice(0,10)},null,2));

}

// Residual spacing extension. Legacy evidence remains a separate immutable history.
const residualFields=['what','mechanism','interfaces','rust','port','priority_reason','dependency_text'];
const residualWords=new Set(('all main a an the and or to from after before at by with than under over of in is has for through timeout default defaults maximum minimum limit within exactly including lines line exit above below between every first last least most only otherwise channel version width height depth rows times uses bounds remaining requires allows accepts exceeds reaches caps capped waits one two up down size range mode key keys value values length count counts initial start port return returns step wait plus minus takes keeps ceilings ceiling cost type stop seconds window budget defaulting using around roughly '+
'actually adaptive add adds age aligned allocate alone always analogous applies approximate are arrays as attempt attempts backward batch be become becomes beginning beyond binding block bounded brightness button byte bytes callback cap capacity center checks chmod clamped clamps clipped close closes code collapsed columns combined command common compaction complete component configured confirm conservatively consumes content context converted counted created crossing cursor debounce debounced debounces decay dedicated defines deleting description differs dir directories disabled dispatcher dissolve divisible downloads duplicate each else embeds emits enables ends entry error even every exact exceed exception exchange exclusive exits expanded expect expiry exported factor fade failure file fixed flags floor forced fresh full gap generates generation glitch grace gray grow header heat helpers hides high hue idle implementation improve improved include indentation indented index indices inspected integer integers interval into its keyof keypad label labels lengths level limits lists low machine margin marker matching max medium min minimal misidentifies movement must name narrow newest nodes non normal notices now number numeric occupy occupying offer offset opens optional ordinary orientation original outer output own pad padding page palette path payload per percentages permits pinned planes prefix prefixes preserve preview previews priority private privately produces project properties quality ranges reach reaching read readonly recognizes regressions remains reminders render renders repetitions repulsion requests required requirements reserve reserves resettable results retention returning revision roots route row run safety same sanitized saved saying scale score section separate settled shared shells show shows shrink signal sleep sleeping slice source squared standard state status statuses strict sub substring subtract subtracts suffix supports tag tail targets temperature test tests then thread threshold timer tool top total truncates truncating turn until unusual use usual versus vertical visible visits weights wheel widths world zoom remain merges native model queue refresh claims gate versions headings runtime thinking col object quota artifact fit card mapped independent evals cases ledger surfaces').split(/\s+/));
const residualSuffixes=new Set(('ms s second seconds minute minutes hour hours byte bytes bit bits char chars character characters symbol symbols state states entry entries MiB KiB GiB px fps hex million lowercase lowercasehex random each nodes rows columns records errors area areas ledger original redacted same pattern candidate active cells code units terminal firstpass themes profiles accepted items result UTF unique runtime type reviewed mapped excluded').split(/\s+/));
const residualExact=[
 ['§11a670','§11a 670','AUTH-007 report section followed by line range'],
 ['§11a676','§11a 676','AUTH-011 report section followed by line range'],
 ['optionalvalidation','optional validation','SUB-017 ordinary adjective and noun'],
 ['andtime','and time','SUB-017 ordinary conjunction and noun'],
 ['anduse','and use','SUB-017 ordinary conjunction and verb'],
 ['enabledfalse','enabled false','COG-055 parameter label followed by its false value'],
 ['orhyphen','or hyphen','SUB-006 ordinary conjunction followed by a prose noun'],
 ['897mapped26excluded','897 mapped 26 excluded','RESUME historical candidate counts, without changing the values'],
 ['807runtime847type','807 runtime 847 type','RESUME historical export counts, without changing the values'],
 ['gh3','gh 3','SUB-006 command name followed by retained command-token count'],
 ['pip2','pip 2','SUB-006 command name followed by retained command-token count'],
 ['flash128','flash 128','AI-036 model shorthand followed by numeric budget'],
 ['Marked18','Marked 18','MISC-017 library name followed by its pinned version'],
 ['Vitest4','Vitest 4','MISC-040 library name followed by its pinned version'],
 ['Highlight.js11','Highlight.js 11','MISC-017 library name followed by its pinned version'],
 ['tool_call600','tool_call 600','EXT-021 parameter name followed by its 600-second deadline'],
 ['context_window0','context_window 0','AUTH-008 parameter name followed by its default value'],
 ['Spacer1','Spacer 1','TUI component name followed by its one-row spacing value'],
 ['Box1','Box 1','TUI component name followed by horizontal/vertical padding values'],
 ['Text0','Text 0','TUI component name followed by horizontal/vertical padding values'],
 ['defaultlimit100','default limit 100','ordinary noun phrase and numeric limit'],
 ['lowercasehex','lowercase hex','ordinary prose phrase'],
 ['andcycles','and cycles','ordinary conjunction and noun'],
 ['anddurably','and durably','ordinary conjunction and adverb'],
 ['returnsfalse','returns false','ordinary return-value prose'],
 ['acceptMISC','accept MISC','RESUME prose action and area label'],
 ['integrateAI019','integrate AI019','RESUME prose action and entry label'],
 ['beforeAI019','before AI019','RESUME prose relation and entry label']
];
const residualCite=/\b(?:ice|rust|rust-active):[A-Za-z0-9_./@+\-]+:\d+(?:[-–]\d+)?/g;
const knownTechnical=/^(?:v\d+|[ui](?:8|16|32|64|128)|f(?:16|32|64|128)|(?:UTF|utf|base|sha|SHA|OSC|SGR|CSI|SS|X|F|P|G|V|U|B|C|M)\d+\w*|float(?:16|32|64)|arm64|x64|x86|uuidv7|bm25|e2e|fs2|System32|rundll32|iterm2|iTerm2|p50|p75|p90|p99|h[1-6])$/;
function protectedRanges(text,context={}){
 const ranges=[];
 const add=(start,end,kind)=>{if(end>start)ranges.push({start,end,kind,text:text.slice(start,end)});};
 const collect=(rx,kind)=>{for(const m of text.matchAll(rx))add(m.index,m.index+m[0].length,kind);};
 // Generated source metadata mirrors the canonical source field, which is outside prose-edit scope.
 if(context.field==='markdown'&&areas.some(area=>context.artifact===area+'.md'))collect(/^### 2\. ice source\r?\n[\s\S]*?(?=^### 3\. What it does\r?$|(?![\s\S]))/gm,'rendered-source-metadata');
 collect(/(^|\n)[ \t]*(\x60{3,}|~{3,})[^\n]*\n[\s\S]*?\n[ \t]*\2[^\n]*/g,'fence');
 collect(/\x60+[^\x60]*\x60+/g,'inline-code');
 collect(/\bCOLORTERM=truecolor\/24bit\b|\bdisabled\/30s\/1m\/other\b/g,'literal-config-value');
 collect(/\bnvidia\/nemotron-3-super-120b-a12b\b/g,'model-identifier');
 collect(/(?<![A-Z_])[A-Z][A-Z_]*\d+[A-Z0-9_]*(?![A-Z0-9_])/g,'uppercase-identifier');
 collect(/https?:\/\/[^\s\x60<>()|]+/g,'url');
 collect(/\b(?:ice|rust|rust-active):[A-Za-z0-9_./@+\-]+:\d+(?:[-–]\d+)?/g,'citation');
 collect(/\]\([^\n)]+\)/g,'markdown-link-target');
 collect(/(?:~?\/(?:home|usr|tmp|workspace|var|etc)\/[A-Za-z0-9_./@+\-]+|(?:\b(?:crates|packages|scripts|docs|evidence|src|tools)\/)[A-Za-z0-9_./@+\-]+(?::\d+(?:[-–]\d+)?)?)/g,'path');
 for(const m of text.matchAll(/\b[A-Za-z0-9_.-]+\.(?:rs|ts|tsx|mjs|js|json|jsonl|toml|md|csv|py)(?::?\d+(?:[-–]\d+)?)?/g)){const library=text.startsWith('Highlight.js11.9.0',m.index);add(m.index,m.index+(library?'Highlight.js'.length:m[0].length),library?'library-name':'file-or-source-abbreviation');}
 collect(/\b(?:rpc-mode|rpc\.test|state-machine)\d+(?:[-–]\d+)?/g,'source-abbreviation');
 collect(/(?<![\w])#[0-9a-fA-F]{3,8}\b|(?<![0-9])0x[0-9a-fA-F]+\b|\b(?=[0-9a-fA-F]*[a-fA-F])[0-9a-fA-F]{7,64}\b/g,'hex-or-hash');
 collect(/\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b/g,'uuid');
 collect(/(?<![0-9])\d+\.\d+\.\d+(?:[-+][A-Za-z0-9.-]+)?/g,'version');
 collect(/(?<![\w])"(?:\\.|[^"\\\n])*"(?![\w])/g,'quoted-literal');
 collect(/(?<![\w])'(?:\\.|[^'\\\n])*'(?![\w])/g,'quoted-literal');
 collect(/(?<![\w])\/(?=[^/\n]*[\\[\]^$])(?:\\.|[^/\s])+\/[dgimsuvy]*/g,'regex-literal');
 collect(/\^(?:\\.|[^\n$])+\$/g,'regex-expression');
 collect(/\[(?:\\.|[A-Za-z0-9_+.^\\:-]){2,}\]/g,'character-class-or-marker');
 collect(/\b(?:claude\d+(?:\.\d+)?\w*|(?:opus|sonnet|fable|qwen|Gemini|Gemma)\d+(?:\.\d+)?|Iv1\.[A-Za-z0-9]+|k0valik|sting8k)\b/g,'model-or-public-identifier');
 // Balanced calls and shape literals are code even when their author omitted delimiters.
 for(let i=0;i<text.length;i++){
  const ch=text[i],isCall=ch==='('&&/[A-Za-z0-9_]/.test(text[i-1]??'')&&/[A-Za-z_][A-Za-z0-9_]*(?:(?:\.|::)[A-Za-z_][A-Za-z0-9_]*)*$/.test(text.slice(Math.max(0,i-160),i))&&!/\barity$/.test(text.slice(Math.max(0,i-160),i));
  if(ch!=='{'&&!isCall)continue;
  const opener=ch,closer=ch==='{'?'}':')';let depth=1,j=i+1,quote=null;
  for(;j<text.length&&j-i<50000;j++){
   const c=text[j];if(quote){if(c==='\\'){j++;continue;}if(c===quote)quote=null;continue;}
   if(c==='"'||c==="'"){quote=c;continue;}
   if(c===opener)depth++;if(c===closer&&--depth===0)break;
   if(isCall&&c==='\n')break;
  }
  if(depth===0){const begin=isCall?i-(text.slice(Math.max(0,i-160),i).match(/[A-Za-z_][A-Za-z0-9_.:]*$/)?.[0].length??0):i;const shape=text.slice(begin,j+1);if(/^\{response,\s*session\}$/.test(shape)){add(begin+shape.indexOf('response'),begin+shape.indexOf('response')+8,'shape-identifier');add(begin+shape.indexOf('session'),begin+shape.indexOf('session')+7,'shape-identifier');}else add(begin,j+1,isCall?'code-call':'shape-literal');i=j;}
 }
 ranges.sort((a,b)=>a.start-b.start||b.end-a.end);
 const merged=[];for(const r of ranges){const last=merged.at(-1);if(last&&r.start<last.end){if(r.end>last.end){last.end=r.end;last.text=text.slice(last.start,last.end);last.kind+='+'+r.kind;}}else merged.push({...r});}
 return merged;
}
function residualDetail(text,context={}){
 if(typeof text!=='string')return {text,changes:[],protected:[]};
 const spans=protectedRanges(text,context),insertions=new Map();
 const inside=p=>spans.some(r=>p>r.start&&p<r.end);
 const add=(p,kind,before,after,reason)=>{if(inside(p)||/\s/.test(text[p-1]??'')||/\s/.test(text[p]??''))return;if(!insertions.has(p))insertions.set(p,{offset:p,kind,before,after,reason});};
 const id=context.entry_id??context.id??'';
 for(const m of text.matchAll(/(?<![A-Za-z0-9_])([A-Za-z]+)(?=\d)/g)){
  const w=m[1],p=m.index+w.length;
  if(!residualWords.has(w.toLowerCase())||w===w.toUpperCase())continue;
  if(w.toLowerCase()==='or'&&text[m.index-1]==='-'&&/^0\b/.test(text.slice(p)))continue;
  add(p,'word-number',w,w+' ','Reviewed ordinary prose word or numeric parameter label');
 }
 for(const [before,after,reason]of residualExact){
  const esc=before.replace(/[.*+?^$(){}|[\]\\]/g,'\\$&');
  for(const m of text.matchAll(new RegExp('(?<![A-Za-z0-9_])'+esc+'(?![A-Za-z0-9_])','g'))){
   let ai=0;for(let bi=0;bi<before.length;bi++,ai++){while(after[ai]===' '){add(m.index+bi,'prose-glue',before,after,reason);ai++;}}
  }
 }
 for(const m of text.matchAll(/\b[A-Za-z]+'s(?=\d)/g))add(m.index+m[0].length,'possessive-number',m[0],m[0]+' ','Possessive prose immediately preceding a numeric quantity');
 for(const m of text.matchAll(/(\d+(?:\.\d+)?)([A-Za-z]+)/g)){
  const start=m.index,p=start+m[1].length,suffix=m[2];
  if(start>0&&/[A-Za-z0-9_]/.test(text[start-1])&&!insertions.has(start))continue;
  if(!residualSuffixes.has(suffix))continue;
  add(p,'number-unit-or-word',m[0],m[1]+' '+suffix,'Reviewed numeric quantity followed by a unit or ordinary prose word');
  if(suffix==='lowercasehex')add(p+9,'prose-glue','lowercasehex','lowercase hex','Ordinary lowercase hexadecimal prose');
 }
 for(const m of text.matchAll(/,(?=[A-Za-z])/g))add(m.index+1,'comma-space',',',', ','Prose comma outside code, paths, quotes and citations');
 const changes=[...insertions.values()].sort((a,b)=>a.offset-b.offset).map(c=>({...c,context:text.slice(Math.max(0,c.offset-65),Math.min(text.length,c.offset+85))}));
 let result='',cursor=0;for(const c of changes){result+=text.slice(cursor,c.offset)+' ';cursor=c.offset;}result+=text.slice(cursor);
 return {text:result,changes,protected:spans};
}
export function normalizeResidualProse(text,context={}){return residualDetail(text,context).text;}
export const normalizeResidualSpacing=normalizeResidualProse;
function scopedIdentifiers(text,context,exportNames){
 const detail=residualDetail(text,context),splits=new Set(detail.changes.filter(c=>c.kind!=='comma-space').map(c=>c.offset));
 const tokens=[];
 for(const m of text.matchAll(/\b[A-Za-z_][A-Za-z0-9_]*\b/g)){
  const raw=m[0],parts=[];let start=0;
  for(let i=1;i<raw.length;i++)if(splits.has(m.index+i)){parts.push(raw.slice(start,i));start=i;}parts.push(raw.slice(start));
  for(const t of parts){if(!/^[A-Za-z_]/.test(t)||residualSuffixes.has(t)||residualWords.has(t.toLowerCase()))continue;
   if(exportNames.has(t)||/_|[a-z][A-Z]|\d/.test(t))tokens.push(t);
  }
 }
 return tokens.sort();
}

const manifestProseFields=new Set([...residualFields,'notes','finding','capability','ts_comparison','question','reason','rationale','description','claim','unverified','open_questions']);
function residualRun(mode){
 const checking=mode==='residual-check',previewing=mode==='residual-preview',applying=mode==='residual-apply';
 if(!checking&&!previewing&&!applying)throw Error('Unknown residual mode');
 if(!checking&&read('progress.json').complete)throw Error('Completed audit protected from historical mutation');
 const now=new Date().toISOString(),toolSha=sha(fs.readFileSync(fileURLToPath(import.meta.url))),dryPath=path.join(E,'spacing-residual-dry-run.json');
 const exportNames=new Set(Object.values(read('qa-unnamed-exports-2026-10-20.json').symbols).flat().map(r=>r.symbol));
 const historicalNames=['spacing-review.json','spacing-validation.json'];
 const historical=historicalNames.map(name=>({artifact:'evidence/'+name,sha256:sha(fs.readFileSync(path.join(E,name)))}));
 const report={schema_version:2,captured_at:now,owner:'/root/residual_spacing',mode,source_only:true,applied:false,tool_sha256:toolSha,scope:{spec_fields:residualFields,accepted_manifest_prose_fields:[...manifestProseFields],markdown:['README.md','evidence/RESUME.md'],check_includes_rendered_areas:checking},historical_spacing_proofs:historical,historical_dry_run_incident:'evidence/lane-native-final-spacing-preview-incident.json',inputs:[],field_changes:[],changes:[],proofs:[],identifier_diffs:[],citation_diffs:[],protected_span_mismatches:[],idempotence_failures:[],intentional_exclusions:[],outputs:[]};
 const jobs=[],same=(a,b)=>JSON.stringify(a)===JSON.stringify(b),allBefore={citations:[],identifiers:[],protected:[]},allAfter={citations:[],identifiers:[],protected:[]};
 function processText(before,context){
  const detail=residualDetail(before,context),after=detail.text,afterRanges=protectedRanges(after,context),oldProtected=detail.protected.map(r=>r.text),newProtected=afterRanges.map(r=>r.text);
  const bc=[...before.matchAll(residualCite)].map(m=>m[0]),ac=[...after.matchAll(residualCite)].map(m=>m[0]);
  const bi=scopedIdentifiers(before,context,exportNames),ai=scopedIdentifiers(after,context,exportNames);
  if(!same(bc,ac))report.citation_diffs.push(context);
  if(!same(bi,ai))report.identifier_diffs.push({...context,removed:bi.filter(t=>!ai.includes(t)),added:ai.filter(t=>!bi.includes(t))});
  if(!same(oldProtected,newProtected))report.protected_span_mismatches.push({...context,before:oldProtected.map(sha),after:newProtected.map(sha)});
  if(normalizeResidualProse(after,context)!==after)report.idempotence_failures.push(context);
  report.proofs.push({...context,citations_before_sha256:sha(JSON.stringify(bc)),citations_after_sha256:sha(JSON.stringify(ac)),identifiers_before_sha256:sha(JSON.stringify(bi)),identifiers_after_sha256:sha(JSON.stringify(ai)),protected_before_sha256:sha(JSON.stringify(oldProtected)),protected_after_sha256:sha(JSON.stringify(newProtected))});
  allBefore.citations.push(...bc);allAfter.citations.push(...ac);allBefore.identifiers.push(...bi);allAfter.identifiers.push(...ai);allBefore.protected.push(...oldProtected);allAfter.protected.push(...newProtected);
  if(detail.changes.length){
   const fieldIndex=report.field_changes.length;report.field_changes.push({...context,before,after,before_sha256:sha(before),after_sha256:sha(after),change_indexes:[]});
   for(const c of detail.changes){const index=report.changes.length;report.changes.push({index,field_change_index:fieldIndex,...context,...c});report.field_changes[fieldIndex].change_indexes.push(index);}
  }
  for(const m of after.matchAll(/\b[A-Za-z_]+[0-9]+[A-Za-z0-9_]*/g)){
   const p=m.index,token=m[0],span=afterRanges.find(r=>p>=r.start&&p<r.end);
   if(span||knownTechnical.test(token)||/_|[a-z][A-Z]/.test(token)||!residualWords.has((token.match(/^[A-Za-z]+/)??[''])[0].toLowerCase())){
    report.intentional_exclusions.push({...context,token,offset:p,reason:span?'Protected '+span.kind:knownTechnical.test(token)?'Recognized technical identifier or version':/_|[a-z][A-Z]/.test(token)?'Code-like identifier; no reviewed ordinary-word boundary':'Not a reviewed ordinary prose prefix; retained identifier/model/source shorthand',context:after.slice(Math.max(0,p-35),p+token.length+55)});
   }
  }
  return after;
 }
 const currentManifestSections=new Set(['entries','claims','rust_only','open_questions','unverified','candidate_exclusions','dependency_requests','dependency_resolutions']);
 const preservedManifestSections=new Set(['read_evidence','read_sources','source_review','provenance','reference_normalization','source_baseline','shared_read_evidence','independent_qa','owner_qa_before_acceptance','acceptance','verification','targeted_supplements','supplemental_read_dependencies']);
 const pointerEscape=k=>String(k).replaceAll('~','~0').replaceAll('/','~1');
 function walkManifest(node,artifact,pointer='',parentKey='',entryId=null){
  if(typeof node==='string')return manifestProseFields.has(parentKey)?processText(node,{artifact,entry_id:entryId,field:parentKey,pointer}):node;
  if(Array.isArray(node))return node.map((v,i)=>walkManifest(v,artifact,pointer+'/'+i,parentKey,entryId));
  if(node&&typeof node==='object'){const id=node.id??node.entry_id??entryId;for(const[k,v]of Object.entries(node)){if((pointer===''&&!currentManifestSections.has(k))||preservedManifestSections.has(k)||/(?:_history|_before_)/.test(k))continue;node[k]=walkManifest(v,artifact,pointer+'/'+pointerEscape(k),k,id);}}return node;
 }
 for(const area of areas){
  const artifact='evidence/spec-'+area+'.json',full=path.join(B,artifact),raw=fs.readFileSync(full,'utf8'),spec=JSON.parse(raw);report.inputs.push({artifact,sha256:sha(raw)});
  for(const e of spec.entries)for(const field of residualFields)if(typeof e[field]==='string')e[field]=processText(e[field],{artifact,entry_id:e.id,field});
  const after=JSON.stringify(spec,null,2)+'\n';if(after!==raw)jobs.push({artifact,raw,after,area});
  const ma='evidence/'+area+'.json',mp=path.join(B,ma);
  if(fs.existsSync(mp)){const mr=fs.readFileSync(mp,'utf8');report.inputs.push({artifact:ma,sha256:sha(mr)});const mv=walkManifest(JSON.parse(mr),ma),mo=JSON.stringify(mv,null,2)+'\n';if(mo!==mr)jobs.push({artifact:ma,raw:mr,after:mo});}
  if(checking){const md=area+'.md',mdRaw=fs.readFileSync(path.join(B,md),'utf8');report.inputs.push({artifact:md,sha256:sha(mdRaw)});processText(mdRaw,{artifact:md,entry_id:null,field:'markdown'});}
 }
 for(const artifact of ['README.md','evidence/RESUME.md']){
  const raw=fs.readFileSync(path.join(B,artifact),'utf8');report.inputs.push({artifact,sha256:sha(raw)});
  const after=processText(raw,{artifact,entry_id:null,field:'markdown'});if(after!==raw)jobs.push({artifact,raw,after});
 }
 report.summary={changes:report.changes.length,changed_fields:report.field_changes.length,changed_entries:new Set(report.changes.map(c=>c.entry_id).filter(Boolean)).size,changed_artifacts:new Set(report.changes.map(c=>c.artifact)).size,by_kind:Object.fromEntries([...new Set(report.changes.map(c=>c.kind))].map(k=>[k,report.changes.filter(c=>c.kind===k).length])),protected_span_mismatches:report.protected_span_mismatches.length,identifier_diffs:report.identifier_diffs.length,citation_diffs:report.citation_diffs.length,idempotence_failures:report.idempotence_failures.length,remaining_candidates_intentionally_excluded:report.intentional_exclusions.length};
 report.citations_byte_identical=!report.citation_diffs.length;report.identifiers_byte_identical=!report.identifier_diffs.length;report.protected_spans_byte_identical=!report.protected_span_mismatches.length;
 report.inventory_hashes={citations_before:sha(JSON.stringify(allBefore.citations)),citations_after:sha(JSON.stringify(allAfter.citations)),identifiers_before:sha(JSON.stringify(allBefore.identifiers)),identifiers_after:sha(JSON.stringify(allAfter.identifiers)),protected_before:sha(JSON.stringify(allBefore.protected)),protected_after:sha(JSON.stringify(allAfter.protected))};
 report.remaining_real_prose_glue=checking?report.changes.length:report.idempotence_failures.length;
 report.passed=report.citations_byte_identical&&report.identifiers_byte_identical&&report.protected_spans_byte_identical&&!report.idempotence_failures.length&&(!checking||!report.changes.length);
 if(previewing)fs.writeFileSync(dryPath,JSON.stringify(report,null,2)+'\n');
 if(applying){
  const prior=read('spacing-residual-dry-run.json'),approvalPath=path.join(E,'lane-residual-spacing-review.json'),approval=read('lane-residual-spacing-review.json');
  if(!report.passed||!prior.passed)throw Error('Residual protected-evidence/idempotence proof failed');
  if(approval.approved!==true||approval.dry_run_sha256!==sha(fs.readFileSync(dryPath))||approval.tool_sha256!==toolSha)throw Error('Residual approval does not bind this dry run and tool');
  if(!same(approval.reviewed_change_indexes,prior.changes.map(c=>c.index)))throw Error('Every residual change must be individually reviewed');
  if(!same(prior.inputs,report.inputs)||!same(prior.changes,report.changes))throw Error('Residual inputs or exact reviewed changes drifted');
  for(const j of jobs)fs.writeFileSync(path.join(B,j.artifact),j.after);
  for(const area of [...new Set(jobs.map(j=>j.area).filter(Boolean))]){const renderer=fs.existsSync(path.join(B,'tools/render-area.mjs'))?path.join(B,'tools/render-area.mjs'):path.join(B,'render-area.mjs');execFileSync(process.execPath,[renderer,area],{stdio:'pipe'});}
  for(const h of historical)if(sha(fs.readFileSync(path.join(B,h.artifact)))!==h.sha256)throw Error('Historical spacing evidence changed');
  for(const j of jobs){const current=fs.readFileSync(path.join(B,j.artifact),'utf8');if(current!==j.after)throw Error('Residual apply bytes differ from review');report.outputs.push({artifact:j.artifact,sha256:sha(current)});}
  report.applied=true;report.applied_at=new Date().toISOString();report.dry_run_sha256=sha(fs.readFileSync(dryPath));report.approval_sha256=sha(fs.readFileSync(approvalPath));report.non_prose_structure_byte_identical=true;report.remaining_real_prose_glue=0;
  fs.writeFileSync(path.join(E,'spacing-residual-validation.json'),JSON.stringify(report,null,2)+'\n');
 }
 const result={schema_version:2,mode,passed:report.passed,applied:report.applied,summary:report.summary,citations_byte_identical:report.citations_byte_identical,identifiers_byte_identical:report.identifiers_byte_identical,protected_spans_byte_identical:report.protected_spans_byte_identical,remaining_real_prose_glue:report.remaining_real_prose_glue,inventory_hashes:report.inventory_hashes,identifier_diffs:report.identifier_diffs.slice(0,20),protected_span_mismatches:report.protected_span_mismatches.slice(0,8),idempotence_failures:report.idempotence_failures.slice(0,8),dry_run_sha256:previewing?sha(fs.readFileSync(dryPath)):report.dry_run_sha256,tool_sha256:toolSha,...(checking?{inputs:report.inputs,remaining_changes:report.changes}: {})};
 console.log(JSON.stringify(result,null,2));if(!report.passed)process.exitCode=1;
}
const invokedDirectly=process.argv[1]&&path.resolve(process.argv[1])===fileURLToPath(import.meta.url);
if(invokedDirectly){const requested=process.argv[2]??'preview';if(requested.startsWith('residual-'))residualRun(requested);else runLegacy();}
