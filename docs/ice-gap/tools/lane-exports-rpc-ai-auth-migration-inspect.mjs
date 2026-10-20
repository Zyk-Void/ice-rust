// Read-only preparation inventory for relocating audit helpers.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';
const base=path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const hash=b=>crypto.createHash('sha256').update(b).digest('hex');
function walk(dir){return fs.readdirSync(dir,{withFileTypes:true}).flatMap(e=>e.isDirectory()?walk(path.join(dir,e.name)):[path.join(dir,e.name)]);}
const mode=process.argv[2];
if(mode==='inventory'){
 const files=walk(base).filter(p=>/\.(mjs|py)$/.test(p)).sort();
 const rows=files.slice(Number(process.argv[3]??0),Number(process.argv[4]??files.length)).map(p=>{const raw=fs.readFileSync(p);const text=raw.toString('utf8');const lines=text.split('\n');const terms=/\/home\/|ice-gap|import |require\(|from |write|append|rename|unlink|mkdir|copy|spawn|exec|subprocess|read.*coverage|process\.argv|sys\.argv|__file__|import\.meta|process\.cwd|chdir|complete|independent-review/;
 const snippets=lines.flatMap((line,i)=>terms.test(line)?[{line:i+1,text:line}]:[]);
 return {path:path.relative(base,p),bytes:raw.length,sha256:hash(raw),lines:lines.length-(text.endsWith('\n')?1:0),snippets};});
 console.log(JSON.stringify(rows));
}else if(mode==='metadata'){
 for(const rel of ['evidence/progress.json','evidence/final-validation.json','evidence/baseline.json','evidence/active-checkout-source-map.json']){
  const raw=fs.readFileSync(path.join(base,rel));const data=JSON.parse(raw);
  const out={path:rel,sha256:hash(raw),keys:Object.keys(data),metadata:{}};
  for(const [k,v] of Object.entries(data))if(v===null||typeof v!=='object'||['identities','ts','rust','counts','summary','comparison','frozen_manifest'].includes(k))out.metadata[k]=v;else out.metadata[k]=Array.isArray(v)?{array_length:v.length}:{keys:Object.keys(v)};
  console.log(JSON.stringify(out));
 }
}else throw Error('inventory or metadata required');
