import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
assertAuditOpen();
// Audit-only local Git provenance. No application, tests, builds, fetch or source writes.
import fs from 'node:fs';
import path from 'node:path';
import {createHash} from 'node:crypto';
import {execFileSync,spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
const base=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const root=path.resolve(base,'../..');
const git=args=>execFileSync('git',['--no-optional-locks',...args],{cwd:root,encoding:'utf8',maxBuffer:32*1024*1024});
const upstream=git(['rev-parse','0fb2d89']).trim();
const upstreamPaths=git(['ls-tree','-r','--name-only',upstream,'--','crates']).trim().split('\n');
const catalog=JSON.parse(fs.readFileSync(path.join(base,'evidence/rust-defects.json'),'utf8'));
const ancestral=new Map(),versions=new Map();
const normalize=s=>s.trim().replace(/\b(?:rpi|pi|icers)(?=[-_])/g,'ice');
const code=s=>{const t=s.trim();return t.length>=8&&!/^(?:\/\/|\*|#\[|[{};]+$)/.test(t);};
const ancestor=sha=>{if(!ancestral.has(sha))ancestral.set(sha,sha==='0000000000000000000000000000000000000000'?false:spawnSync('git',['--no-optional-locks','merge-base','--is-ancestor',sha,upstream],{cwd:root}).status===0);return ancestral.get(sha);};
const version=p=>{if(!versions.has(p))versions.set(p,git(['show',upstream+':'+p]));return versions.get(p);};
const report={captured_at:new Date().toISOString(),root,head:git(['rev-parse','HEAD']).trim(),upstream_commit:upstream,remotes:git(['remote','-v']).trim().split('\n'),tag_matches:git(['tag','--points-at',upstream]).trim().split('\n').filter(Boolean),upstream_cargo:git(['show',upstream+':Cargo.toml']),method:'Read-only local Git blame and upstream text comparison. Line-text matches and ancestry are evidence for manual review, not automatic proof of identical behavior. No application execution.',targets:[]};
for(const d of catalog)for(const t of d.provenance_targets??[]){
const rel=t.file;
if(path.isAbsolute(rel)||rel.split('/').includes('..'))throw Error('Unsafe source path');
const bytes=fs.readFileSync(path.join(root,rel));const lines=bytes.toString('utf8').split('\n');const[a,b]=t.range;if(a<1||b<a||b>lines.length)throw Error('Invalid range '+rel);
const current=lines.slice(a-1,b);const blame=git(['blame','--line-porcelain','-L',t.range.join(','),'--',rel]);
const commits=[...new Set([...blame.matchAll(/^([a-f0-9]{40}) \d+ \d+/gm)].map(m=>m[1]))].map(sha=>({sha,in_upstream:ancestor(sha)}));
const row={id:d.id,entry_ids:d.entry_ids,file:rel,range:t.range,current_sha256:createHash('sha256').update(bytes).digest('hex'),current_excerpt:current.map((l,i)=>(a+i)+' | '+l).join('\n'),blame_commits:commits,upstream_matches:[]};
const significant=current.filter(code).map(normalize);
for(const p of upstreamPaths.filter(p=>p.endsWith('/'+path.basename(rel)))){
const content=version(p);const oldLines=content.split('\n'),oldNorm=oldLines.map(normalize);const matches=significant.filter(s=>oldNorm.includes(s));
if(!matches.length)continue;
let anchor=significant.find(s=>s.length>=24&&oldNorm.filter(x=>x===s).length===1)??matches.find(s=>s.length>=12)??matches[0];let at=oldNorm.indexOf(anchor);if(at<0){anchor=matches[0];at=oldNorm.indexOf(anchor);}
const first=Math.max(0,at-8),last=Math.min(oldLines.length,at+35);
row.upstream_matches.push({path:p,sha256:createHash('sha256').update(content).digest('hex'),matched_significant_lines:matches.length,current_significant_lines:significant.length,anchor,range:[first+1,last],excerpt:oldLines.slice(first,last).map((l,i)=>(first+i+1)+' | '+l).join('\n')});
}
row.upstream_matches.sort((x,y)=>y.matched_significant_lines-x.matched_significant_lines);report.targets.push(row);
}
const out=path.join(base,'evidence/rust-defect-provenance.json');if(fs.existsSync(out)&&!fs.existsSync(path.join(base,'evidence/rust-defect-provenance-before-closeout.json')))fs.copyFileSync(out,path.join(base,'evidence/rust-defect-provenance-before-closeout.json'));
fs.writeFileSync(out,JSON.stringify(report,null,2)+'\n');
console.log(JSON.stringify({artifact:'evidence/rust-defect-provenance.json',root,head:report.head,upstream_commit:upstream,targets:report.targets.length,summary:report.targets.map(t=>({id:t.id,file:t.file,range:t.range,blame_commits:t.blame_commits,upstream_matches:t.upstream_matches.map(x=>({path:x.path,matched:x.matched_significant_lines,total:x.current_significant_lines,range:x.range}))}))},null,2));
