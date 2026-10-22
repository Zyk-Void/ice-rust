import { auditHelperBase } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
// Tools-area read-only reader: verifies current source hashes against coordinator baseline before displaying ranges.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {execFileSync} from 'node:child_process';
const base=auditHelperBase;
const manifest=JSON.parse(fs.readFileSync(path.join(base,'evidence/resume-source-identities-2026-10-19T222055Z.json'),'utf8'));
for(const arg of process.argv.slice(2)){
 const m=/^(ice|rust):(.+):(\d+)-(\d+)$/.exec(arg);if(!m)throw Error('Bad reference');
 const root=manifest.identities[m[1]].root,p=path.resolve(root,m[2]);if(!p.startsWith(root+path.sep))throw Error('Outside repository');
 const buf=fs.readFileSync(p);if(buf.includes(0))throw Error('Binary source');
 const sha=crypto.createHash('sha256').update(buf).digest('hex');
 let expected=manifest.hashes[m[1]][m[2]],basis='coordinator source manifest';
 const supplemental=JSON.parse(fs.readFileSync(path.join(base,'evidence/tools-supplemental-source-hashes.json'),'utf8'));
 if(!expected&&supplemental.hashes[m[1]]?.[m[2]]){expected=supplemental.hashes[m[1]][m[2]];basis='current dirty-test snapshot; historical content hash unavailable';}
 if(!expected){const git=execFileSync('git',['show',manifest.identities[m[1]].head+':'+m[2]],{cwd:root,maxBuffer:20*1024*1024});expected=crypto.createHash('sha256').update(git).digest('hex');basis='clean tracked Git revision (not original manifest)';}
 if(sha!==expected)throw Error('Source drift '+arg+' '+sha+' expected '+expected);
 const lines=buf.toString('utf8').split('\n'),s=Number(m[3]),e=Math.min(Number(m[4]),lines.length);
 console.log('\n### '+arg+' total_lines='+lines.length+' sha256='+sha+' verified='+basis);
 for(let i=s-1;i<e;i++)console.log(String(i+1).padStart(6)+' | '+lines[i]);
}
