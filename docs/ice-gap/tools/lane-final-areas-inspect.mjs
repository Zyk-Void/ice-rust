import fs from 'node:fs';import path from 'node:path';import {fileURLToPath} from 'node:url';
const E=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../evidence');
const [mode,...args]=process.argv.slice(2);const read=n=>JSON.parse(fs.readFileSync(path.join(E,n),'utf8'));
if(mode==='meta')for(const n of args){const x=read(n);console.log(JSON.stringify({file:n,type:Array.isArray(x)?'array':typeof x,keys:Object.keys(x),summary:Object.fromEntries(Object.entries(x).filter(([k,v])=>!Array.isArray(v)&&typeof v!=='object')),arrays:Object.fromEntries(Object.entries(x).filter(([k,v])=>Array.isArray(v)).map(([k,v])=>[k,{length:v.length,first:v[0]}]))}));}
else if(mode==='part'){const[n,key,start='0',count='50']=args;const x=read(n);console.log(JSON.stringify((key?x[key]:x)?.slice?.(+start,+start+ +count)??x[key]??x,null,2));}
else if(mode==='entries'){const[n,...ids]=args;for(const e of read(n).entries.filter(e=>!ids.length||ids.includes(e.id)))console.log(JSON.stringify(e));}
else if(mode==='index'){for(const n of args)console.log(JSON.stringify({file:n,entries:read(n).entries.map(e=>({id:e.id,title:e.title,source:e.source}))}));}
else throw Error('mode');
