import { auditHelperBase, auditHelperNative } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
if(["read","integrity"].includes(process.argv[2]))assertAuditOpen();
// Deterministic evidence reader. No repository module execution, network, tests or agents.
// Source files are never written. Evidence output is confined to this audit directory.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {execFileSync} from 'node:child_process';
const roots={ice:'/home/mewtwo/Zks/ice',rust:auditHelperNative};
const out=auditHelperBase;
const [command,...args]=process.argv.slice(2);
function resolve(ref){const m=/^(ice|rust):(.+)$/.exec(ref);if(!m)throw Error('Expected ice: or rust:');const p=path.resolve(roots[m[1]],m[2]);if(!p.startsWith(roots[m[1]]+path.sep))throw Error('Outside source roots');if(m[2].split('/').some(x=>['node_modules','.git'].includes(x)))throw Error('Excluded dependency or Git-internal read');return p;}
function read(ref){const p=resolve(ref),buf=fs.readFileSync(p);if(buf.includes(0))throw Error('Binary source');const text=buf.toString('utf8');return{ref,sha256:crypto.createHash('sha256').update(buf).digest('hex'),bytes:buf.length,lines:text.split('\n'),text};}
function save(rel,value){const p=path.resolve(out,rel);if(!p.startsWith(out+path.sep))throw Error('Output boundary');fs.mkdirSync(path.dirname(p),{recursive:true});fs.writeFileSync(p,typeof value==='string'?value:JSON.stringify(value,null,2)+'\n');}
if(command==='read'){
 const outputs=[];
 for(const arg of args){const m=/^(ice|rust):(.+?)(?::(\d+)-(\d+))?$/.exec(arg);if(!m)throw Error('Invalid range '+arg);const r=read(m[1]+':'+m[2]);const a=m[3]?+m[3]:1,b=m[4]?Math.min(+m[4],r.lines.length):r.lines.length;if(a<1||a>b)throw Error('Range outside file '+arg);const header={path:r.ref,sha256:r.sha256,bytes:r.bytes,total_lines:r.lines.length,returned:[a,b]};outputs.push(header);console.log('\n### '+JSON.stringify(header));console.log(r.lines.slice(a-1,b).map((s,i)=>(a+i)+' | '+s).join('\n'));}
 save('evidence/direct-last-read.json',{at:new Date().toISOString(),outputs,note:'Range returned, not an assertion of semantic verification.'});
}else if(command==='integrity'){
 const ledger=JSON.parse(fs.readFileSync(path.join(out,'evidence/tracked-files.json'),'utf8'));const mismatches=[];
 for(const row of ledger){try{const buf=fs.readFileSync(resolve('ice:'+row.path));const actual=crypto.createHash('sha256').update(buf).digest('hex');if(actual!==row.sha256)mismatches.push({path:row.path,baseline:row.sha256,actual});}catch(e){mismatches.push({path:row.path,error:e.message});}}
 const data={at:new Date().toISOString(),rows:ledger.length,mismatches,heads:Object.fromEntries(Object.entries(roots).map(([k,v])=>[k,execFileSync('git',['-C',v,'rev-parse','HEAD'],{encoding:'utf8'}).trim()])),note:'Hashes confirm file identity only, not previous connector-read content or authored semantics.'};save('evidence/direct-integrity.json',data);console.log(JSON.stringify(data,null,2));
}else if(command==='files'){
 const [side,...patterns]=args;if(!roots[side])throw Error('Unknown side');const ps=execFileSync('git',['-C',roots[side],'ls-files','-z','--',...patterns],{encoding:'utf8'}).split('\0').filter(Boolean);
 for(const p of ps){try{const r=read(side+':'+p);console.log([r.lines.length,r.sha256,p].join('\t'));}catch(e){console.log('unreadable\t'+p+'\t'+e.message);}}
}else if(command==='find'){
 const [needle,...refs]=args;const re=new RegExp(needle,'u');for(const ref of refs){const r=read(ref);console.log('### '+r.ref+' SHA256 '+r.sha256);for(let i=0;i<r.lines.length;i++)if(re.test(r.lines[i]))console.log((i+1)+' | '+r.lines[i]);}
}else throw Error('Unknown command');
