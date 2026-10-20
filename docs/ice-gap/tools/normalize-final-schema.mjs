// Coordinator-only mechanical schema repair; preserves every original supporting reference.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {fileURLToPath} from 'node:url';
import {execFileSync} from 'node:child_process';
const base=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..'), ev=path.join(base,'evidence');
const areas=['ai','agent','tools','session','settings','extensions','rpc-remote','tui','subagents','safe-verify','observability','cognee','blackhole','providers-auth','cli-packaging','misc'];
const mode=process.argv[2]||'preview';
if(!['preview','apply'].includes(mode))throw Error('preview or apply');
const read=n=>JSON.parse(fs.readFileSync(path.join(ev,n),'utf8'));
if(read('progress.json').complete)throw Error('Refusing mutation of a published complete audit.');
const sha=x=>crypto.createHash('sha256').update(x).digest('hex');
const changes=[], planned=[];
const prefix=/\b(ice|rust):([A-Za-z0-9_./@+\-]+):(\d+(?:[-–]\d+)?)(?:((?:,\s*\d+(?:[-–]\d+)?)+))/g;
function expand(text,loc){return text.replace(prefix,(old,side,p,first,rest)=>{const replacement=[first,...rest.split(',').map(x=>x.trim()).filter(Boolean)].map(r=>side+':'+p+':'+r).join('; ');changes.push({kind:'expanded_citation',location:loc,before:old,after:replacement});return replacement;});}
function walk(x,loc){if(typeof x==='string')return expand(x,loc);if(Array.isArray(x))return x.map((v,i)=>walk(v,loc+'['+i+']'));if(x&&typeof x==='object')return Object.fromEntries(Object.entries(x).map(([k,v])=>[k,walk(v,loc+'.'+k)]));return x;}
function queue(name,before,obj){const after=JSON.stringify(obj,null,2)+'\n';if(after!==before)planned.push({name,before_sha256:sha(before),after_sha256:sha(after),after});}
for(const area of areas){
 const name=area+'.json', before=fs.readFileSync(path.join(ev,name),'utf8'), m=JSON.parse(before);
 for(let i=0;i<(m.surfaces??[]).length;i++){const s=m.surfaces[i];for(const [field,side] of [['ice_ref','ice'],['rust_ref','rust']]){
  const old=s[field];if(!old||/^[A-Za-z0-9_./@+\-]+:\d+$/.test(old))continue;
  const qualified=old.startsWith(side+':')?old:side+':'+old;
  const matches=[...qualified.matchAll(/\b(ice|rust):([A-Za-z0-9_./@+\-]+):(\d+)(?:[-–](\d+))?/g)];
  if(!matches.length||matches[0][1]!==side)throw Error('Cannot normalize '+area+' '+field+' '+old);
  const leftover=qualified.replace(/\b(ice|rust):[A-Za-z0-9_./@+\-]+:\d+(?:[-–]\d+)?/g,'').replace(/[;\s]/g,'');
  if(leftover)throw Error('Unparsed surface reference '+old);
  const replacement=matches[0][2]+':'+matches[0][3];
  s.supporting_references??={};s.supporting_references[field]=matches.map(m=>m[0]);
  s.reference_normalization??={};s.reference_normalization[field]={original:old,method:'first source line in the single-anchor CSV field; complete references retained in supporting_references'};
  s[field]=replacement;
  changes.push({kind:'surface_primary_anchor',location:area+'.surfaces['+i+'].'+field,name:s.name,entry_id:s.entry_id,before:old,after:replacement,supporting_references:s.supporting_references[field]});
 }}
 m.claims=walk(m.claims??[],name+'.claims');queue(name,before,m);
}
const n='rust-defects.json', b=fs.readFileSync(path.join(ev,n),'utf8');queue(n,b,walk(JSON.parse(b),n));
const report={schema_version:1,captured_at:new Date().toISOString(),mode,source_reads_claimed:false,reason:'Complete gate requires one path:line in each CSV anchor and fully qualified disjoint ranges; behavior and mappings remain unchanged.',changes,files:planned.map(({after,...x})=>x)};
if(mode==='apply'){
 const preview=read('final-schema-normalization-preview.json');
 if(JSON.stringify(preview.files)!==JSON.stringify(report.files))throw Error('Inputs changed after reviewed preview');
 for(const f of planned){if(sha(fs.readFileSync(path.join(ev,f.name)))!==f.before_sha256)throw Error('Stale '+f.name);}
 for(const f of planned)fs.writeFileSync(path.join(ev,f.name),f.after);
 const helper=fs.existsSync(path.join(base,'merge-progress.mjs'))?path.join(base,'merge-progress.mjs'):path.join(base,'tools','merge-progress.mjs');
 execFileSync(process.execPath,[helper],{cwd:path.resolve(base,'../..'),stdio:'pipe'});
}
fs.writeFileSync(path.join(ev,'final-schema-normalization-'+mode+'.json'),JSON.stringify(report,null,2)+'\n');
console.log(JSON.stringify({mode,files:planned.length,changes:changes.length,kinds:Object.fromEntries([...new Set(changes.map(x=>x.kind))].map(k=>[k,changes.filter(x=>x.kind===k).length])),sample:changes.slice(0,5)},null,2));
