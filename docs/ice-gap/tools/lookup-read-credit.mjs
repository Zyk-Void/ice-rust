// Read-only lookup of already recorded source review; no application execution.
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
const base=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../evidence');
const queries=process.argv.slice(2);
if(!queries.length)throw Error('Pass source-path substrings');
const primary=['ai','agent','tools','session','settings','extensions','rpc-remote','tui','subagents','safe-verify','observability','cognee','blackhole','providers-auth','cli-packaging','misc'].map(x=>x+'.json');
const extra=fs.readdirSync(base).filter(n=>n.endsWith('.json')&&/source-review|read-manifest|test-closure|read-progress|test-coverage|coordinator|native-.*supplement/.test(n)&&!n.startsWith('spec-'));
const results=[];
for(const name of [...new Set([...primary,...extra])]){
 if(!fs.existsSync(path.join(base,name)))continue;
 let obj;try{obj=JSON.parse(fs.readFileSync(path.join(base,name),'utf8'));}catch{continue;}
 function visit(v,loc){
  if(!v||typeof v!=='object')return;
  if(Array.isArray(v)){v.forEach((x,i)=>visit(x,loc+'['+i+']'));return;}
  const p=typeof v.path==='string'?v.path:typeof v.file==='string'?v.file:null;
  if(p&&queries.some(q=>p.includes(q))&&(v.ranges||v.range||v.start_line||v.total_lines||v.sha256))results.push({artifact:'evidence/'+name,record_location:loc,...v});
  for(const[k,x]of Object.entries(v))if(x&&typeof x==='object')visit(x,loc+'.'+k);
 }
 visit(obj,'$');
}
console.log(JSON.stringify({queries,matches:results},null,2));
