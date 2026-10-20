import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
const base=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..'),ev=path.join(base,'evidence');
const areas=['ai','agent','tools','session','settings','extensions','rpc-remote','tui','subagents','safe-verify','observability','cognee','blackhole','providers-auth','cli-packaging','misc'];
const read=n=>JSON.parse(fs.readFileSync(path.join(ev,n),'utf8')), args=process.argv.slice(2);
if(args[0]==='brief2'){
  for(const n of args.slice(1)){const o=read(n);console.log(JSON.stringify({file:n,keys:Object.keys(o),arrays:Object.fromEntries(Object.entries(o).filter(([,v])=>Array.isArray(v)).map(([k,v])=>[k,{count:v.length,shapes:[...new Set(v.map(x=>x&&typeof x==='object'?Object.keys(x).sort().join(','):typeof x))]}])),objects:Object.fromEntries(Object.entries(o).filter(([,v])=>v&&typeof v==='object'&&!Array.isArray(v)).map(([k,v])=>[k,{keys:Object.keys(v)}]))}));}
}else if(args[0]==='exclusions'){
  const out=[];
  function walk(x,loc,parentPath,art){
    if(!x||typeof x!=='object')return;
    const p=x.path||x.file||parentPath;
    const own=Object.fromEntries(Object.entries(x).filter(([k,v])=>/(exclude|redact|unverified|unread|blocked|strict_visible)/i.test(k)&&v!==null&&v!==false&&!(Array.isArray(v)&&!v.length)));
    if(Object.keys(own).length)out.push({artifact:art,loc,path:p,fields:own});
    if(Array.isArray(x))x.forEach((v,i)=>walk(v,loc+'['+i+']',p,art));
    else for(const[k,v]of Object.entries(x))if(!['source','text','excerpt','declaration'].includes(k))walk(v,loc+'.'+k,k.startsWith('ice:')||k.startsWith('rust:')?k:p,art);
  }
  for(const n of args.slice(1))walk(read(n),'root',null,n);
  console.log(JSON.stringify(out));
}else if(args[0]==='records'){
  const o=read(args[1]),src=args[2]&&args[2]!=='-'?args[2].split('.').reduce((v,k)=>v?.[k],o):o;
  const arr=Array.isArray(src)?src:Object.entries(src).map(([k,v])=>({key:k,value:v}));
  console.log(JSON.stringify(arr.slice(+args[3]||0,args[4]?(+args[3]||0)+(+args[4]):undefined)));
}else if(args[0]==='legacy-summary'){
 const old=read('read-coverage.json'),files=old.files||old,accepted=new Set(areas.flatMap(a=>(read(a+'.json').read_evidence||[]).map(r=>r.path)));
 console.log(JSON.stringify({count:Object.keys(files).length,missing_sha256:Object.entries(files).filter(([k,v])=>k.includes(':')&&!v.sha256).map(([k,v])=>({path:k,...v})),only_legacy:Object.entries(files).filter(([k])=>k.includes(':')&&!accepted.has(k)).map(([k,v])=>({path:k,...v}))}));
}else if(args[0]==='read-records'){
 const query=args[1],out=[];
 for(const a of areas)for(const r of read(a+'.json').read_evidence||[])if(r.path?.includes(query))out.push({artifact:a+'.json',record:r});
 const old=read('read-coverage.json');for(const[k,v]of Object.entries(old.files||old))if(k.includes(query))out.push({artifact:'read-coverage.json',path:k,record:v});
 console.log(JSON.stringify(out));
}else if(args[0]==='files')console.log(JSON.stringify(fs.readdirSync(ev).filter(n=>n.endsWith('.json'))));
else if(args[0]==='dirty-tests'){
 const b=read('baseline.json'),m=read('resume-source-identities-2026-10-19T222055Z.json'),out=[];
 for(const row of b.ts.status.split('\n')){const p=row.slice(3);if(!p.includes('/test/')&&!p.includes('/tests/'))continue;
 const refs=[];for(const a of areas)for(const r of read(a+'.json').read_evidence||[])if(r.path==='ice:'+p)refs.push({artifact:a+'.json',record:r});
 out.push({path:p,status:row.slice(0,2),baseline_manifest:m.hashes?.ice?.[p],records:refs});
 }console.log(JSON.stringify(out));
}
else if(args[0]==='document'){
 const f=path.join(base,args[1]);if(path.dirname(f)!==base)throw new Error('Audit-root documents only');const text=fs.readFileSync(f,'utf8').split('\n'),out=[];for(let i=2;i<args.length;i+=2)out.push(text.slice(+args[i]-1,+args[i+1]).map((s,j)=>(j+(+args[i]))+' | '+s).join('\n'));console.log(out.join('\n'));
}
else throw new Error('Unknown operation');
