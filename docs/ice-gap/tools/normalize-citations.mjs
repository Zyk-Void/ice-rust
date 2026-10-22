// Requested citation expansion and prose spacing only. No source files are modified.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {fileURLToPath} from 'node:url';
import {execFileSync} from 'node:child_process';
const base=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const areas=['ai','agent','tools','session','settings','extensions','rpc-remote','tui','subagents','safe-verify','observability','cognee','blackhole','providers-auth','cli-packaging','misc'];
const [mode='preview']=process.argv.slice(2);
if(!['preview','apply'].includes(mode))throw Error('Expected preview or apply');
const sha=s=>crypto.createHash('sha256').update(s).digest('hex');
const read=n=>JSON.parse(fs.readFileSync(path.join(base,'evidence',n),'utf8'));
const report={captured_at:new Date().toISOString(),mode,fields:[],unresolved_short_citations:[],changes:[],inputs:[],summary:{}};
const words='a|an|the|and|or|to|from|after|before|at|by|with|than|under|over|of|in|is|has|for|through|timeout|default|maximum|minimum|limit|within|exactly|including|lines|line|exit|above|below|between|every|first|last|least|most|only|otherwise|channel|version|width|height|depth|rows|times|uses|bounds|remaining|requires|allows|accepts|exceeds|reaches|caps|capped|waits';
const wordNum=new RegExp('(?<![#/._-])\\b('+words+')(?=\\d)','gi');
const protectedParts=/(\x60{3}[\s\S]*?\x60{3}|\x60[^\x60]*\x60|https?:\/\/[^\s)]+|(?:ice|rust):[^\s;]+:\d+(?:[-–]\d+)?)/g;
function revise(text,id,field,transform){
 return text.replace(transform.re,(...args)=>{const before=args[0],after=transform.fn(...args);if(before!==after)report.changes.push({id,field,kind:transform.kind,before,after});return after;});
}
function refs(text,id,field,anchors){
 let t=revise(text,id,field,{kind:'expand-citation-ranges',re:/\b((?:ice|rust):[A-Za-z0-9_./@+\-]+):(\d+(?:[-–]\d+)?(?:,\s*\d+(?:[-–]\d+)?)+)/g,fn:(m,p,r)=>r.split(/,\s*/).map(x=>p+':'+x.replace('–','-')).join('; ')});
 t=revise(t,id,field,{kind:'explicit-repository-prefix',re:/(?<![\w/:])((?:crates|packages)\/[A-Za-z0-9_./@+\-]+):(\d+(?:[-–]\d+)?)/g,fn:(m,p,r)=>(p.startsWith('crates/')?'rust:':'ice:')+p+':'+r.replace('–','-')});
 t=revise(t,id,field,{kind:'expand-local-citation',re:/(?<![\w/:\-.])([A-Za-z_][A-Za-z0-9_-]*\.(?:rs|ts|mjs)):(\d+(?:[-–]\d+)?(?:,\s*\d+(?:[-–]\d+)?)*)/g,fn:(m,n,r)=>{const candidates=[...(anchors.get(n)??[])];if(!candidates.length&&n.endsWith('.rs')){const exact=explicitNative[id+':'+n];if(exact)candidates.push(exact);else if(uniqueNative.get(n)?.size===1)candidates.push(...uniqueNative.get(n));}if(candidates.length===1)return r.split(/,\s*/).map(x=>candidates[0]+':'+x.replace('–','-')).join('; ');report.unresolved_short_citations.push({id,field,citation:m,candidates});return m;}});
 return t;
}
function prose(text,id,field){
 return text.split(protectedParts).map((part,index)=>{
  if(index%2)return part;
  let t=revise(part,id,field,{kind:'space-word-number',re:wordNum,fn:m=>m+' '});
  t=revise(t,id,field,{kind:'space-number-unit',re:/\b(\d+(?:\.\d+)?)(ms|s|seconds?|minutes?|hours?|bytes?|bits?|chars?|symbols|states|entries|MiB|KiB|px|fps|hex)(?=\b)/g,fn:(m,n,u)=>n+' '+u});
  t=revise(t,id,field,{kind:'space-prose-qualifier',re:/\b(\d+)lowercasehex\b/g,fn:(m,n)=>n+' lowercase hex'});
  return t;
 }).join('');
}
const inventory=read('rust-tracked-files.json').map(x=>typeof x==='string'?x:x.path);
const uniqueNative=new Map();
for(const p of inventory){const n=path.basename(p);if(!uniqueNative.has(n))uniqueNative.set(n,new Set());uniqueNative.get(n).add('rust:'+p);}
const explicitNative={
 'SET-021:session.rs':'rust:crates/ice-cli/src/session.rs',
 'SET-023:session.rs':'rust:crates/ice-cli/src/session.rs',
 'TUI-228:image.rs':'rust:crates/ice-tui/src/image.rs'
};
const outputs=[];
for(const area of areas){
 const name='spec-'+area+'.json',raw=fs.readFileSync(path.join(base,'evidence',name),'utf8'),s=JSON.parse(raw);
 report.inputs.push({area,name,sha256:sha(raw)});
 for(const e of s.entries){
  const anchors=new Map();
  for(const v of Object.values(e))if(typeof v==='string')for(const m of v.matchAll(/\b((?:ice|rust):[A-Za-z0-9_./@+\-]+):\d+/g)){const n=path.basename(m[1]);if(!anchors.has(n))anchors.set(n,new Set());anchors.get(n).add(m[1]);}
  for(const [k,v]of Object.entries(e))if(typeof v==='string'){
   let next=refs(v,e.id,k,anchors);
   // Spacing is handled independently by fix-spacing.mjs with preservation proofs.
   if(next!==v){report.fields.push({area,id:e.id,field:k,before_sha256:sha(v),after_sha256:sha(next)});e[k]=next;}
  }
  if(!/\brust:[A-Za-z0-9_./@+\-]+:\d+/.test(e.rust)){
   const native=[...e.source.matchAll(/\brust:[A-Za-z0-9_./@+\-]+:\d+(?:-\d+)?/g)].map(m=>m[0]);
   if(native.length){const append=' Native comparison source: '+[...new Set(native)].map(x=>'\x60'+x+'\x60').join('; ')+'.';e.rust+=append;report.changes.push({id:e.id,field:'rust',kind:'repeat-native-anchor-in-required-field',before:'',after:append});}
  }
 }
 outputs.push({area,name,s,raw,content:JSON.stringify(s,null,2)+'\n'});
}
report.summary={areas:areas.length,changed_fields:report.fields.length,replacements:report.changes.length,by_kind:Object.fromEntries([...new Set(report.changes.map(x=>x.kind))].map(k=>[k,report.changes.filter(x=>x.kind===k).length])),unresolved_short_citations:report.unresolved_short_citations.length};
if(mode==='apply'){
 const p=read('progress.json');if(p.complete)throw Error('Completed audit is protected from historical mutation; use an explicit reviewed revision.');
 for(const a of areas)if(!fs.existsSync(path.join(base,'evidence',a+'.json')))throw Error('Wait for authoritative handoff/publication: '+a);
 for(const o of outputs){
  fs.writeFileSync(path.join(base,'evidence',o.name),o.content);
  for(const n of[o.area+'.json','proposal-'+o.area+'.json']){
   const file=path.join(base,'evidence',n);if(!fs.existsSync(file))continue;const m=read(n);
   m.entries=m.entries.map(old=>{const current=o.s.entries.find(e=>e.id===old.id);if(!current)throw Error('Unknown metadata entry');return Object.fromEntries(Object.entries(old).map(([k,v])=>[k,k in current?current[k]:v]));});
   fs.writeFileSync(file,JSON.stringify(m,null,2)+'\n');
  }
  const renderer=fs.existsSync(path.join(base,'tools','render-area.mjs'))?path.join(base,'tools','render-area.mjs'):path.join(base,'render-area.mjs');
  execFileSync(process.execPath,[renderer,o.area],{encoding:'utf8'});
 }
 fs.writeFileSync(path.join(base,'evidence','citation-normalization.json'),JSON.stringify(report,null,2)+'\n');
}
fs.writeFileSync(path.join(base,'evidence','citation-normalization-'+mode+'.json'),JSON.stringify(report,null,2)+'\n');
console.log(JSON.stringify({...report,fields:undefined,inputs:undefined,changes:report.changes.slice(0,10)},null,2));
