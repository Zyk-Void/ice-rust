// Extend only missing rows of the captured checkout hunk map. Audit evidence writes only.
import fs from 'node:fs'; import path from 'node:path'; import crypto from 'node:crypto'; import {execFileSync} from 'node:child_process'; import {fileURLToPath} from 'node:url';
const B=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..'),E=path.join(B,'evidence');
const read=n=>JSON.parse(fs.readFileSync(path.join(E,n),'utf8')),sha=x=>crypto.createHash('sha256').update(x).digest('hex');
if(read('progress.json').complete)throw Error('Completed audit protected');
const m=read('active-checkout-source-map.json'),d=read('active-checkout-diff-hunks.json');
const old=m.identities.old,active=m.identities.active;
const git=(root,args)=>execFileSync('git',['--no-optional-locks','-C',root,...args],{encoding:'utf8',maxBuffer:40*1024*1024,stdio:['ignore','pipe','pipe'],env:{...process.env,GIT_TERMINAL_PROMPT:'0'}});
for(const x of[old,active])if(git(x.root,['rev-parse','HEAD']).trim()!==x.head||git(x.root,['status','--porcelain=v1','--untracked-files=no']).trim())throw Error('Checkout drift, retain existing map and review identity first');
const unsafe=p=>!p||path.isAbsolute(p)||p.split('/').includes('..')||/^(?:\.ice|\.icers|\.ssh|\.aws|\.git)\//.test(p)||/(^|\/)\.env(?:\.|$)/.test(p);
const existing=new Map(d.rows.map(r=>[r.old_path,r])), missing=m.rows.filter(r=>r.old?.kind==='text'&&r.active?.kind==='text'&&!['identical','renamed_identical'].includes(r.status)&&!existing.has(r.old_path)&&!unsafe(r.old_path)&&!unsafe(r.active_path));
const parse=raw=>{const hs=[];let h;for(const line of raw.split('\n')){const x=/^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@(.*)$/.exec(line);if(x){h={old_start:+x[1],old_count:x[2]===undefined?1:+x[2],active_start:+x[3],active_count:x[4]===undefined?1:+x[4],section:x[5].trim(),lines:[]};hs.push(h);}else if(h&&/^[ +-]/.test(line))h.lines.push(line);}return hs;};
const rows=[...d.rows],added=[];
for(const r of missing){
 const raw=git(active.root,['diff','--no-ext-diff','--no-textconv','--no-color','--find-renames=50%','-U0',old.head,active.head,'--',...new Set([r.old_path,r.active_path])]);
 const changes=parse(raw),next={...r,unified_diff:raw,diff_sha256:sha(raw),changes:changes.map(({lines,...h})=>h),captured_by:'root-missing-row-extension'};
 rows.push(next);added.push({old_path:r.old_path,active_path:r.active_path,hunks:changes.length});
}
const out={schema_version:2,captured_at:new Date().toISOString(),source_only:true,old_root:old.root,active_root:active.root,identities:m.identities,original_map:{path:'evidence/active-checkout-diff-hunks.json',sha256:sha(fs.readFileSync(path.join(E,'active-checkout-diff-hunks.json'))),captured_rows:d.rows.length},source_map_sha256:sha(fs.readFileSync(path.join(E,'active-checkout-source-map.json'))),method:'Retained every captured row verbatim and fetched only missing safe text-file diff rows from local Git at the same fixed commits. Hunk acquisition is metadata, not manual source review.',rows,added_rows:added,excluded_paths:m.rows.filter(r=>!['identical','renamed_identical','active_only'].includes(r.status)&&unsafe(r.old_path)).map(r=>r.old_path),status:'captured-pending-manual-overlap-review'};
fs.writeFileSync(path.join(E,'active-checkout-diff-hunks-complete.json'),JSON.stringify(out,null,2)+'\n');
console.log(JSON.stringify({retained: d.rows.length,added:added.length,total:rows.length,crate_files:rows.filter(r=>r.old_path?.startsWith('crates/')).length,crate_hunks:rows.filter(r=>r.old_path?.startsWith('crates/')).reduce((n,r)=>n+r.changes.length,0),excluded:out.excluded_paths},null,2));
