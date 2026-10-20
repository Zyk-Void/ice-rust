// Inspect semantic review queues without altering audited source or evidence.
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
const base=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const names=['safe-verify','cognee','blackhole','agent','extensions','tools','session','subagents','observability','settings','ai','providers-auth','rpc-remote','cli-packaging','misc','tui'];
const [mode,areaFilter]=process.argv.slice(2);
const out=[];
for(const area of names){const f=path.join(base,'evidence','spec-'+area+'.json');if(!fs.existsSync(f)||areaFilter&&area!==areaFilter)continue;const spec=JSON.parse(fs.readFileSync(f,'utf8'));
 for(const e of spec.entries){
  const first=e.mechanism.split(/\n\s*\n/)[0];
  const clean=first.replace(/\x60[^\x60]*\x60/g,'CODE');
  const sentences=(clean.match(/[.!?](?:\s+|$)/g)??[]).length;
  if(mode==='openings'&&(sentences<2||/^(?:\x60|[-*] |\d+[.)] )/.test(first)))out.push({area,id:e.id,title:e.title,what:e.what,first,sentences});
  if(mode==='spacing'){for(const k of ['what','mechanism','rust','port','priority_reason','dependency_text']){const t=(e[k]??'').replace(/\x60[^\x60]*\x60/g,'');for(const m of t.matchAll(/\b(?:a|an|the|and|or|to|from|after|before|at|by|with|than|under|over|of|in|is|has|for|through|timeout|default|maximum|minimum|limit|within|exactly)(?:\d[\w.-]*)(?=\s|[,;:.])/gi))out.push({area,id:e.id,field:k,token:m[0],context:t.slice(Math.max(0,m.index-60),m.index+m[0].length+70)});}}
 }
}
console.log(JSON.stringify(out,null,2));
