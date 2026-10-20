// Read-only audit reconciliation inspection. No application code is executed.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {fileURLToPath} from 'node:url';
const base=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const areas=['ai','agent','tools','session','settings','extensions','rpc-remote','tui','subagents','safe-verify','observability','cognee','blackhole','providers-auth','cli-packaging','misc'];
const read=n=>JSON.parse(fs.readFileSync(path.join(base,'evidence',n),'utf8'));
const records=areas.flatMap(a=>{try{return[{area:a,manifest:read(a+'.json'),spec:read('spec-'+a+'.json')}]}catch(e){if(e.code==='ENOENT')return[];throw e;}});
const mode=process.argv[2]??'summary';
const ids=new Set(records.flatMap(x=>x.spec.entries.map(e=>e.id)));
if(mode==='summary'){
 console.log(JSON.stringify(records.map(({area,manifest:m,spec:s})=>({area,entries:s.entries.length,manifest_sha256:crypto.createHash('sha256').update(fs.readFileSync(path.join(base,'evidence',area+'.json'))).digest('hex'),read_count:m.read_evidence?.length??0,read_shapes:[...new Set((m.read_evidence??[]).map(r=>Object.keys(r).sort().join(',')))],supplemental_ledger:m.supplemental_ledger?.length??0,claims:m.claims?.length??0,claim_shapes:[...new Set((m.claims??[]).map(r=>Object.keys(r).sort().join(',')))],rust_only:m.rust_only?.length??0,rust_only_shapes:[...new Set((m.rust_only??[]).map(r=>Object.keys(r).sort().join(',')))],open_questions:m.open_questions??[],dependency_requests:m.dependency_requests??[]})),null,2));
}else if(mode==='catalog'){
 const selected=process.argv.slice(3);
 const all=areas.flatMap(a=>{try{return [{area:a,spec:read('spec-'+a+'.json')}]}catch(e){if(e.code==='ENOENT')return[];throw e;}});
 console.log(JSON.stringify(all.filter(x=>!selected.length||selected.includes(x.area)).map(({area,spec})=>({area,entries:spec.entries.map(e=>({id:e.id,title:e.title,dependencies:e.dependencies??[]}))}))));
}else if(mode==='read-diff'){
 const legacy=read('read-coverage.json'), accepted=new Map();
 for(const {area,manifest:m}of records)for(const r of m.read_evidence??[]){const old=accepted.get(r.path)??{owners:[],ranges:[]};old.owners.push(area);old.ranges.push(...(r.ranges??[]));accepted.set(r.path,old);}
 const tracked=read('tracked-files.json');
 const onlyLegacy=Object.entries(legacy).filter(([p])=>!accepted.has(p));
 const relevant=n=>fs.existsSync(path.join(base,'evidence',n));
 const names=['extensions-source-review.json','tools-source-review.json','session-source-review.json','qa-session-test-closure.json','settings-read-progress.json','qa-settings-test-closure.json','subagents-source-review.json','observability-read-progress.json','qa-observability-independent.json','qa-tools-coordinator.json','tui-source-review.json','misc-source-review.json'];
 const shape=v=>Array.isArray(v)?{length:v.length,record_keys:[...new Set(v.slice(0,5).map(x=>x&&typeof x==='object'?Object.keys(x).join(','):typeof x))],sample:v.slice(0,1)}:v&&typeof v==='object'?{keys:Object.keys(v),sample:Object.entries(v).slice(0,1)}:v;
 console.log(JSON.stringify({legacy_files:Object.keys(legacy).length,accepted_files:accepted.size,only_legacy_count:onlyLegacy.length,only_legacy:onlyLegacy.map(([p,v])=>({path:p,ranges:v.ranges,area:tracked.find(x=>'ice:'+x.path===p)?.area})),supplements:names.filter(relevant).map(name=>({name,shapes:Object.fromEntries(Object.entries(read(name)).map(([k,v])=>[k,shape(v)]))}))},null,2));
}else if(mode==='dependencies'){
 console.log(JSON.stringify(records.flatMap(({area,spec,manifest})=>spec.entries.map(e=>({area,id:e.id,title:e.title,dependencies:e.dependencies??[],dependency_text:e.dependency_text??'',unknown:(e.dependencies??[]).filter(d=>!ids.has(d))}))),null,2));
}else if(mode==='reads'){
 console.log(JSON.stringify(records.filter(r=>!process.argv[3]||r.area===process.argv[3]).map(({area,manifest:m})=>({area,reads:m.read_evidence??[]})),null,2));
}else if(mode==='claims'){
 console.log(JSON.stringify(records.filter(r=>!process.argv[3]||r.area===process.argv[3]).map(({area,manifest:m})=>({area,claims:m.claims??[]})),null,2));
}else if(mode==='matrix'){
 const q=read('qa-first-pass-coverage.json');
 console.log(JSON.stringify({keys:Object.keys(q),shapes:Object.fromEntries(Object.entries(q).map(([k,v])=>[k,Array.isArray(v)?{length:v.length,sample:v.slice(0,2)}:typeof v==='object'&&v?{keys:Object.keys(v),value:JSON.stringify(v).length<4000?v:undefined}:v]))},null,2));
}else if(mode==='evidence-files'){
 console.log(JSON.stringify(fs.readdirSync(path.join(base,'evidence')).filter(n=>/read|coverage|baseline|review|qa|manifest|checkpoint/.test(n)).sort(),null,2));
}else throw Error('Unknown inspection mode');
