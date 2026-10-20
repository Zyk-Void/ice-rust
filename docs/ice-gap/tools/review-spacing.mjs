// Review a distributed sample before the central prose-only spacing mutation.
import fs from 'node:fs';import path from 'node:path';import crypto from 'node:crypto';import{fileURLToPath}from'node:url';
const E=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../evidence'),raw=fs.readFileSync(path.join(E,'spacing-dry-run.json')),q=JSON.parse(raw),selected=new Set();
for(const [kind,n]of[['comma-space',50],['word-number',35],['number-unit',25]]){
 const indices=q.changes.map((c,i)=>c.kind===kind?i:null).filter(x=>x!==null);
 for(let i=0;i<Math.min(n,indices.length);i++)selected.add(indices[Math.floor(i*indices.length/Math.min(n,indices.length))]);
}
const rows=[...selected].sort((a,b)=>a-b).map(i=>({index:i,...q.changes[i]}));
console.log(JSON.stringify({dry_run_sha256:crypto.createHash('sha256').update(raw).digest('hex'),summary:q.summary,rows},null,2));
