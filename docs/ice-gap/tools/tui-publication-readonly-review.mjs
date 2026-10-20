import { auditHelperEvidence } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {fileURLToPath} from 'node:url';
const base=auditHelperEvidence;
const areas=['ai','agent','tools','session','settings','extensions','rpc-remote','tui','subagents','safe-verify','observability','cognee','blackhole','providers-auth','cli-packaging','misc'];
const raw=n=>fs.readFileSync(path.join(base,n),'utf8');
const read=n=>JSON.parse(raw(n));
const sha=n=>crypto.createHash('sha256').update(raw(n)).digest('hex');
const [mode='classify',arg='0',countArg='60']=process.argv.slice(2);
if(mode==='classify'){
 const p=read('proposal-tui.json'), t=read('tracked-files.json');
 const canon=new Set(t.map(r=>typeof r==='string'?r:r.path));
 const rows=(p.supplemental_ledger??p.supplemental_read_dependencies??[]).map((r,index)=>{
  const rp=r.path??r.source_path??'',repo=rp.startsWith('rust:')?'rust':rp.startsWith('ice:')?'ice':rp.startsWith('crates/')?'rust':'ice', pp=rp.replace(/^(ice|rust):/,'');
  const isTest=/(^|\/)(test|tests|__tests__|fixtures)(\/|$)|\.(test|spec)\.[cm]?[jt]sx?$/.test(pp);
  const exists=canon.has(pp)||canon.has(rp);
  return {index,path:rp,canonical_path:pp,repo,is_test:isTest,already_canonical:exists,classification:repo==='rust'?'native-read-only':exists?'already-canonical':isTest?'test-read-only':'new-ts-non-test-dependency',record:r};
 });
 console.log(JSON.stringify({proposal_sha256:sha('proposal-tui.json'),tracked_sha256:sha('tracked-files.json'),tracked_count:t.length,rows_count:rows.length,groups:Object.fromEntries([...new Set(rows.map(r=>r.classification))].map(k=>[k,rows.filter(r=>r.classification===k).length])),rows},null,2));
}else if(mode==='metadata'){
 console.log(JSON.stringify(areas.map(area=>{const s='spec-'+area+'.json', a=area+'.json',p='proposal-'+area+'.json';return {area,spec_sha256:sha(s),spec_keys:Object.keys(read(s)),accepted:fs.existsSync(path.join(base,a))?{path:a,sha256:sha(a),keys:Object.keys(read(a))}:null,proposal:fs.existsSync(path.join(base,p))?{path:p,sha256:sha(p),keys:Object.keys(read(p))}:null};}),null,2));
}else if(mode==='fields'){
 const all=[];
 for(const area of areas){
  const n='spec-'+area+'.json',s=read(n),h=sha(n);
  for(const key of ['introduction','intro','scope','method'])if(typeof s[key]==='string')all.push({file:n,sha256:h,pointer:'/'+key,kind:'introduction',text:s[key]});
  for(let i=0;i<s.entries.length;i++){const e=s.entries[i];if(typeof e.dependency_text==='string')all.push({file:n,sha256:h,pointer:'/entries/'+i+'/dependency_text',id:e.id,kind:'dependency_text',text:e.dependency_text});}
 }
 const start=Number(arg),count=Number(countArg);
 console.log(JSON.stringify({total:all.length,start,fields:all.slice(start,start+count)},null,2));
}else if(mode==='claims'){
 const all=[];
 for(const area of areas){
  const n=fs.existsSync(path.join(base,area+'.json'))?area+'.json':'proposal-'+area+'.json';if(!fs.existsSync(path.join(base,n)))continue;const s=read(n),h=sha(n);
  for(let i=0;i<(s.claims??[]).length;i++){const c=s.claims[i];all.push({file:n,sha256:h,pointer:'/claims/'+i,id:c.id??c.claim_id,claim:c});}
 }
 const start=Number(arg),count=Number(countArg);
 console.log(JSON.stringify({total:all.length,start,claims:all.slice(start,start+count)},null,2));
}else if(mode==='matches'){
 const re=/\b(draft|pending|reserved|unpublished|publication|coordinator|not yet|will attach|after .*assigned|before .*publish)\b/i;
 const all=[];
 for(const area of areas){
  const sname='spec-'+area+'.json',s=read(sname),sh=sha(sname);
  for(const key of ['introduction','intro','scope','method'])if(typeof s[key]==='string'&&re.test(s[key]))all.push({file:sname,sha256:sh,pointer:'/'+key,text:s[key]});
  s.entries.forEach((e,i)=>{if(re.test(e.dependency_text??''))all.push({file:sname,sha256:sh,pointer:'/entries/'+i+'/dependency_text',id:e.id,text:e.dependency_text});});
  const n=fs.existsSync(path.join(base,area+'.json'))?area+'.json':'proposal-'+area+'.json';if(!fs.existsSync(path.join(base,n)))continue;const a=read(n),ah=sha(n);
  (a.claims??[]).forEach((c,i)=>{for(const[k,v]of Object.entries(c))if(typeof v==='string'&&re.test(v))all.push({file:n,sha256:ah,pointer:'/claims/'+i+'/'+k,id:c.id??c.claim_id,text:v});});
 }
 console.log(JSON.stringify({total:all.length,fields:all},null,2));
}else if(mode==='dependency-classification'){
 const paths=['packages/tui/native/darwin/README.md','packages/tui/package.json','packages/tui/README.md'];
 const files=fs.readdirSync(base).filter(n=>n.endsWith('.json')&&!n.startsWith('candidates-'));
 const out=paths.map(p=>({path:p,entry_anchors:[],supplemental_rows:[],read_records:[]}));
 for(const n of files){const d=read(n);for(const o of out){
  if(n.startsWith('spec-')&&!/supplement|fragment|before|terminal/.test(n))for(const e of d.entries??[])if(JSON.stringify(e).includes(o.path))o.entry_anchors.push({file:n,id:e.id,source:e.source});
  if(!n.startsWith('proposal-')&&!n.startsWith('spec-'))for(const r of (Array.isArray(d.supplemental_ledger)?d.supplemental_ledger:[]))if(r.path?.replace(/^ice:/,'')===o.path)o.supplemental_rows.push({file:n,row:r});
  if(/source-review|read-manifest|supplement/.test(n)){const walk=(v,ptr)=>{if(!v||typeof v!=='object')return;if(!Array.isArray(v)&&v.path?.replace(/^ice:/,'')===o.path&&(v.ranges||v.visible_ranges))o.read_records.push({file:n,pointer:ptr,record:v});for(const[k,x]of Object.entries(v))if(x&&typeof x==='object')walk(x,ptr+'/'+k);};walk(d,'');}
 }}
 console.log(JSON.stringify(out,null,2));
}else throw Error('Unknown mode');
