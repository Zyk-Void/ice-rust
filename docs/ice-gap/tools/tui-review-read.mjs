import { auditHelperEvidence } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
// TUI-owned read-only audit helper. Never imports application code or writes source/evidence.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {spawnSync} from 'node:child_process';
const base=auditHelperEvidence;
const roots={ice:'/home/mewtwo/Zks/ice',rust:'/home/mewtwo/Zks/ice-rust'};
const manifest=JSON.parse(fs.readFileSync(path.join(base,'resume-source-identities-2026-10-19T222055Z.json'),'utf8'));
const sha=b=>crypto.createHash('sha256').update(b).digest('hex');
const [mode,...args]=process.argv.slice(2);
function candidateData(){return JSON.parse(fs.readFileSync(path.join(base,'candidates-tui.json'),'utf8'));}
if(mode==='overview'){
 const c=candidateData(),groups={};for(const x of c)groups[x.path]=(groups[x.path]||0)+1;
 const b=fs.readFileSync(path.join(base,'candidates-tui.json'));
 console.log(JSON.stringify({candidates:c.length,sha256:sha(b),groups},null,2));
}else if(mode==='candidates'){
 const c=candidateData();console.log(JSON.stringify(c.slice(Number(args[0]||0),Number(args[1]||c.length)),null,2));
}else if(mode==='candidate-file'){
 const c=candidateData().filter(x=>x.path===args[0]);console.log(JSON.stringify(c,null,2));
}else if(mode==='read'){
 for(const arg of args){
  const m=/^(ice|rust):(.+):(\d+)-(\d+)$/.exec(arg);if(!m)throw Error('Bad ref');
  const file=path.resolve(roots[m[1]],m[2]);if(!file.startsWith(roots[m[1]]+path.sep))throw Error('Outside root');
  const b=fs.readFileSync(file),hash=sha(b),baseline=manifest.hashes[m[1]][m[2]],lines=b.toString('utf8').split('\n');
  if(baseline&&baseline!==hash)throw Error('Source drift '+arg);
  const start=Number(m[3]),end=Math.min(Number(m[4]),lines.length);
  console.log(JSON.stringify({ref:arg,total_lines:lines.length,sha256:hash,baseline:baseline??null,match:baseline?baseline===hash:null}));
  for(let i=start-1;i<end;i++)console.log(String(i+1).padStart(5)+' | '+lines[i]);
 }
}else if(mode==='tests'){
 const revision='3ab05193147f5abb8eb8ee6c0178f6dbce8918b0';
 const output=args.map(p=>{
  if(!/^packages\/(coding-agent|tui)\/test\/.+\.(ts|json)$/.test(p)||p.includes('..'))throw Error('Invalid test path');
  const current=fs.readFileSync(path.join(roots.ice,p)),historical=spawnSync('git',['show',revision+':'+p],{cwd:roots.ice,maxBuffer:1048576});
  const status=spawnSync('git',['status','--short','--',p],{cwd:roots.ice,encoding:'utf8'});if(status.status!==0)throw Error('Git status failed');
  return{path:p,current_sha256:sha(current),revision,revision_sha256:historical.status===0?sha(historical.stdout):null,revision_matches:historical.status===0?sha(current)===sha(historical.stdout):null,status:status.stdout.trim(),historical_read_error:historical.status===0?null:historical.stderr.toString().trim()};
 });console.log(JSON.stringify(output,null,2));
}else throw Error('Unknown mode');
