import { auditHelperEvidence } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
// Terminal-TUI fragment inspection only: no writes, application imports, tests or network commands.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {execFileSync} from 'node:child_process';
const base=auditHelperEvidence;
const manifest=JSON.parse(fs.readFileSync(path.join(base,'resume-source-identities-2026-10-19T222055Z.json'),'utf8'));
const [mode,...args]=process.argv.slice(2);
if(mode==='candidates'){
 const all=JSON.parse(fs.readFileSync(path.join(base,'candidates-tui.json'),'utf8'));
 const owned=all.filter(c=>c.path.startsWith('packages/tui/'));
 console.log(JSON.stringify({total:owned.length,candidates:owned.slice(Number(args[0]||0),Number(args[1]||owned.length))}));
}else if(mode==='read'||mode==='info'){
 for(const ref of args){
  const m=/^(ice|rust):(.+):(\d+)-(\d+)$/.exec(ref);if(!m)throw Error('Bad ref '+ref);
  const root=manifest.identities[m[1]].root,file=path.resolve(root,m[2]);
  if(!file.startsWith(root+path.sep))throw Error('Outside repository');
  const buf=fs.readFileSync(file);if(buf.includes(0))throw Error('Binary source');
  const sha=crypto.createHash('sha256').update(buf).digest('hex');
  let expected=manifest.hashes[m[1]][m[2]],basis='coordinator source manifest';
  if(!expected){
   const git=execFileSync('git',['show',manifest.identities[m[1]].head+':'+m[2]],{cwd:root,maxBuffer:20*1024*1024});
   expected=crypto.createHash('sha256').update(git).digest('hex');basis='clean tracked frozen Git revision; not original manifest';
  }
  const frozenExpected=expected;
  if(sha!==expected){
   const capturedDirtyTest=m[1]==='ice'&&m[2]==='packages/tui/test/editor.test.ts'&&!manifest.hashes.ice[m[2]];
   if(capturedDirtyTest&&sha==='321af6771d3a8a7f96f8cadd13dd0c973230e259a31e82c36765f98103f8b5d1')basis='current supplemental dirty test snapshot; differs from frozen Git '+frozenExpected+';9 added lines; no historical audit snapshot claimed';
   else throw Error('Source drift '+ref+' '+sha+' expected '+expected);
  }
  const lines=buf.toString('utf8').split('\n'),start=Number(m[3]),end=Math.min(Number(m[4]),lines.length);
  if(start<1||start>end)throw Error('Bad range '+ref);
  console.log('META '+JSON.stringify({path:m[1]+':'+m[2],sha256:sha,total_lines:lines.length,range:[start,end],notes:basis,metadata_only:mode==='info'}));
  if(mode==='read')for(let i=start-1;i<end;i++)console.log(String(i+1).padStart(5)+' | '+lines[i]);
 }
}else throw Error('Unknown mode');
