// Read-only closeout inspection; never execute application code.
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
const base=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const areas=['ai','agent','tools','session','settings','extensions','rpc-remote','tui','subagents','safe-verify','observability','cognee','blackhole','providers-auth','cli-packaging','misc'];
const read=n=>JSON.parse(fs.readFileSync(path.join(base,'evidence',n),'utf8'));
const specs=areas.map(area=>({area,spec:read('spec-'+area+'.json')}));
const all=specs.flatMap(({area,spec})=>spec.entries.map(e=>({area,...e})));
const [mode,...args]=process.argv.slice(2);
if(mode==='native-anchors'){
 console.log(JSON.stringify(all.filter(e=>!/(?:rust:)?crates\/[^\s\x60]+:\d+/.test(e.rust||'')).map(e=>({area:e.area,id:e.id,source:e.source,rust:e.rust})),null,2));
}else if(mode==='fields'){
 const requested=new Set(args.join(',').split(','));
 console.log(JSON.stringify(all.filter(e=>requested.has(e.id)).map(e=>({area:e.area,id:e.id,title:e.title,what:e.what,dependencies:e.dependencies,dependency_text:e.dependency_text,rust:e.rust})),null,2));
}else if(mode==='dependency-groups'){
 const d=read('coordinator-dependency-resolutions-draft.json');
 const start=Number(args[0]||0), count=Number(args[1]||50);
 console.log(JSON.stringify(d.records.slice(start,start+count).map((r,i)=>({index:start+i,...r,from_titles:r.from.map(id=>({id,title:all.find(e=>e.id===id)?.title,dependencies:all.find(e=>e.id===id)?.dependencies})),target_titles:r.resolution?.entry_ids?.map(id=>({id,title:all.find(e=>e.id===id)?.title}))})),null,2));
}else if(mode==='spacing'){
 const counts=new Map(), sample=[];
 for(const e of all)for(const k of ['what','mechanism','rust','port','priority_reason','dependency_text']){
 const t=(e[k]??'').replace(/\x60[^\x60]*\x60/g,'');
 for(const m of t.matchAll(/\b(?:a|an|the|and|or|to|from|after|before|at|by|with|than|under|over|of|in|is|has|for|through|timeout|default|maximum|minimum|limit|within|exactly|including)(?:\d[\w.-]*)(?=\s|[,;:.])/gi)){
 counts.set(m[0],(counts.get(m[0])??0)+1);
 if(sample.length<40)sample.push({id:e.id,field:k,token:m[0],context:t.slice(Math.max(0,m.index-35),m.index+m[0].length+45)});
 }}
 console.log(JSON.stringify({tokens:[...counts.entries()],sample},null,2));
}else if(mode==='first-pass-pending'){
 const q=read('qa-first-pass-coverage.json');
 console.log(JSON.stringify({summary:q.summary,validation:q.validation,coverage_complete:q.coverage_complete,uncovered_claims:q.uncovered_claims,follow_up_checks:q.follow_up_checks,draft_claims:q.claims.filter(c=>c.draft_dependent),draft_settings:q.setting_checks.filter(c=>c.draft_dependent),draft_environment:q.environment_checks.filter(c=>c.draft_dependent),active_import_checks:q.active_import_checks},null,2));
}else if(mode==='requests'){
 for(const area of areas){const n=area+'.json';if(fs.existsSync(path.join(base,'evidence',n))){const m=read(n);console.log(JSON.stringify({area,requests:m.dependency_requests??[]},null,2));}}
}else if(mode==='metadata'){
 for(const area of areas){if(args.length&&!args.includes(area))continue;const n=area+'.json';if(fs.existsSync(path.join(base,'evidence',n))){const m=read(n);console.log(JSON.stringify({area,rust_only:m.rust_only??[],open_questions:m.open_questions??[],unverified:m.unverified??[]},null,2));}}
}else if(mode==='defects'){
 const ds=read('rust-defects.json');console.log(JSON.stringify({count:ds.length,summary:ds.map(d=>({id:d.id,title:d.title,severity:d.severity,entry_ids:d.entry_ids})),selected:ds.filter(d=>args.includes(d.id))},null,2));
}else if(mode==='provenance'){
 const q=read('rust-defect-provenance.json');console.log(JSON.stringify(q.targets.filter(t=>args.includes(t.id)),null,2));
}else if(mode==='shape'){
 for(const n of args){const v=read(n);console.log(JSON.stringify({name:n,shape:Array.isArray(v)?{length:v.length,sample:v.slice(0,2)}:Object.fromEntries(Object.entries(v).map(([k,x])=>[k,Array.isArray(x)?{length:x.length,sample:x.slice(0,2)}:x&&typeof x==='object'?{keys:Object.keys(x),value:JSON.stringify(x).length<1200?x:undefined}:x]))},null,2));}
}else throw Error('Unknown mode');
