import { auditHelperEvidence } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
// Subagents-area source reader and actual-read metadata verifier; no writes or application execution.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {execFileSync} from 'node:child_process';
const base=auditHelperEvidence;
const manifest=JSON.parse(fs.readFileSync(path.join(base,'resume-source-identities-2026-10-19T222055Z.json'),'utf8'));
const suppPath=path.join(base,'subagents-supplemental-source-hashes.json');
const supplemental=fs.existsSync(suppPath)?JSON.parse(fs.readFileSync(suppPath,'utf8')):{hashes:{ice:{},rust:{}}};
const metadata=process.argv[2]==='--metadata';
const refs=metadata?JSON.parse(fs.readFileSync(path.join(base,'subagents-read-requests.json'),'utf8')):process.argv.slice(2);
const rows=new Map();
for(const ref of refs){
 const m=/^(ice|rust):(.+):(\d+)-(\d+)$/.exec(ref);if(!m)throw Error('Bad reference '+ref);
 const key=m[1]+':'+m[2],root=manifest.identities[m[1]].root,file=path.resolve(root,m[2]);if(!file.startsWith(root+path.sep))throw Error('Outside repository');
 const buf=fs.readFileSync(file);if(buf.includes(0))throw Error('Binary source');
 const sha=crypto.createHash('sha256').update(buf).digest('hex');
 let expected=manifest.hashes[m[1]][m[2]],basis='coordinator source manifest';
 if(!expected&&supplemental.hashes[m[1]]?.[m[2]]){expected=supplemental.hashes[m[1]][m[2]];basis='current supplemental snapshot; historical content hash unavailable';}
 if(!expected){const git=execFileSync('git',['show',manifest.identities[m[1]].head+':'+m[2]],{cwd:root,maxBuffer:20*1024*1024});expected=crypto.createHash('sha256').update(git).digest('hex');basis='clean tracked Git revision (not original manifest)';}
 if(sha!==expected)throw Error('Source drift '+key+' '+sha+' expected '+expected);
 const lines=buf.toString('utf8').split('\n'),start=Number(m[3]),end=Math.min(Number(m[4]),lines.length);
 if(start<1||start>end)throw Error('Bad range '+ref);
 if(metadata){let row=rows.get(key);if(!row){row={path:key,sha256:sha,total_lines:lines.length,ranges:[],notes:basis+'; source read only, no execution'};rows.set(key,row);}row.ranges.push([start,end]);}
 else{console.log('\n### '+ref+' total_lines='+lines.length+' sha256='+sha+' verified='+basis);for(let i=start-1;i<end;i++)console.log(String(i+1).padStart(6)+' | '+lines[i]);}
}
if(metadata){for(const row of rows.values()){row.ranges.sort((a,b)=>a[0]-b[0]);const merged=[];for(const range of row.ranges){const p=merged.at(-1);if(p&&range[0]<=p[1]+1)p[1]=Math.max(p[1],range[1]);else merged.push(range);}row.ranges=merged;}console.log(JSON.stringify({area:'subagents',complete:false,read_evidence:[...rows.values()]},null,2));}
