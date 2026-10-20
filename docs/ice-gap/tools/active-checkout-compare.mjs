import { auditHelperEvidence } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
// Read-only checkout comparison. Writes no files, imports no application modules, executes no app/tests/builds/network.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
const evidence=auditHelperEvidence;
const frozen=JSON.parse(fs.readFileSync(path.join(evidence,'resume-source-identities-2026-10-19T222055Z.json'),'utf8'));
const roots={old:'/home/mewtwo/Zks/ice-rust',active:'/home/mewtwo/Zykairotis/ice-rust'};
const env={...process.env,GIT_OPTIONAL_LOCKS:'0',GIT_TERMINAL_PROMPT:'0'};
const sha=x=>crypto.createHash('sha256').update(x).digest('hex');
function git(root,args,allow1=false){try{return execFileSync('git',args,{cwd:root,env,encoding:'utf8',maxBuffer:32*1024*1024,stdio:['ignore','pipe','pipe']});}catch(e){if(allow1&&e.status===1)return String(e.stdout??'');throw e;}}
const tracked=root=>git(root,['ls-files','-z']).split('\0').filter(Boolean);
function safePath(root,rel){if(path.isAbsolute(rel)||rel.split('/').includes('..')||rel.startsWith('.git/'))throw Error('Bad tracked path');const abs=path.join(root,rel);return abs;}
function content(root,rel){return /^\.(?:ice|icers)\/skills\/ice-release\/SKILL\.md$/.test(rel)?Buffer.from(git(root,['show','HEAD:'+rel]),'utf8'):fs.readFileSync(safePath(root,rel));}
function file(root,rel){
 const abs=safePath(root,rel);let st;try{st=fs.lstatSync(abs);}catch(e){if(e.code==='ENOENT')return {path:rel,present:false};throw e;}
 if(st.isSymbolicLink()){const target=fs.readlinkSync(abs);return {path:rel,present:true,kind:'symlink',sha256:sha(target),bytes:Buffer.byteLength(target),total_lines:null,target};}
 if(!st.isFile())return {path:rel,present:true,kind:'nonregular',bytes:st.size};
 const real=fs.realpathSync(abs);if(!real.startsWith(root+path.sep))throw Error('Tracked path escapes repository '+rel);
 if(!/^\.(?:ice|icers)\/skills\/ice-release\/SKILL\.md$/.test(rel)&&(/(^|\/)\.env(?:\.|$)/.test(rel)||/^(?:\.ice|\.icers|\.aws|\.ssh)\//.test(rel)))return {path:rel,present:true,kind:'excluded_sensitive_named_path',bytes:st.size};
 const b=content(root,rel),binary=b.includes(0);return {path:rel,present:true,kind:binary?'binary':'text',content_basis:/^\.(?:ice|icers)\//.test(rel)?'tracked HEAD blob; namespace directory content not read':'tracked working-tree bytes',sha256:sha(b),bytes:b.length,total_lines:binary?null:b.toString('utf8').split('\n').length};
}
function origin(root){let s=git(root,['remote','get-url','origin']).trim();return s.replace(/^(https?:\/\/)[^/@]+@/,'$1[redacted-userinfo]@');}
function identity(root){return {root,branch:git(root,['branch','--show-current']).trim()||null,head:git(root,['rev-parse','HEAD']).trim(),origin:origin(root),status_porcelain:git(root,['status','--porcelain=v1','--untracked-files=no']),status_includes_untracked:false};}
function commitChanges(){const raw=git(roots.active,['diff','--name-status','-z','--find-renames=50%',frozen.identities.rust.head,'HEAD','--']);const tok=raw.split('\0').filter(Boolean),rows=[];for(let i=0;i<tok.length;){const status=tok[i++];if(/^[RC]/.test(status))rows.push({status,old_path:tok[i++],active_path:tok[i++]});else{const p=tok[i++];rows.push({status,old_path:status==='A'?null:p,active_path:status==='D'?null:p});}}return rows;}
function category(rel){if(!rel)return 'absent';if(/(^|\/)Cargo\.(toml|lock)$/.test(rel))return 'cargo_manifest';if(rel.startsWith('.github/'))return 'ci';if(/\.(md|rst|txt)$/.test(rel)||rel.startsWith('docs/'))return 'documentation';if(/\.(rs|c|h|cpp|toml|py|sh|mjs|js|ts|tsx|yaml|yml|json)$/.test(rel))return 'source_or_configuration';return 'other_tracked';}
function inventory(){
 const oldNames=tracked(roots.old),newNames=tracked(roots.active),newSet=new Set(newNames),renames=new Map(commitChanges().filter(r=>/^R/.test(r.status)).map(r=>[r.old_path,r.active_path])),used=new Set(),rows=[];
 for(const oldPath of oldNames){const activePath=newSet.has(oldPath)?oldPath:(renames.has(oldPath)&&newSet.has(renames.get(oldPath))?renames.get(oldPath):null);if(activePath)used.add(activePath);const old=file(roots.old,oldPath),active=activePath?file(roots.active,activePath):null;const frozenHash=frozen.hashes.rust[oldPath]??null;rows.push({old_path:oldPath,active_path:activePath,category:category(oldPath),status:!active?'old_only':!old.present||!active.present?'working_tree_missing':old.sha256===active.sha256?(oldPath===activePath?'identical':'renamed_identical'):(oldPath===activePath?'modified':'renamed_modified'),old,active,frozen_source_sha256:frozenHash,frozen_source_matches:frozenHash&&old.sha256?old.sha256===frozenHash:null});}
 for(const activePath of newNames)if(!used.has(activePath))rows.push({old_path:null,active_path:activePath,category:category(activePath),status:'active_only',old:null,active:file(roots.active,activePath),frozen_source_sha256:null,frozen_source_matches:null});
 return rows;
}
function diff(row,unified){if(/^\.(?:ice|icers)\//.test(row.old_path??row.active_path??''))return git(roots.active,['diff','--no-ext-diff','--no-textconv','--no-color','--find-renames=50%','--unified='+unified,frozen.identities.rust.head,'HEAD','--',...[row.old_path,row.active_path].filter(Boolean)]);const a=row.old_path?safePath(roots.old,row.old_path):'/dev/null',b=row.active_path?safePath(roots.active,row.active_path):'/dev/null';return git(roots.old,['diff','--no-index','--no-ext-diff','--no-textconv','--no-color','--text','--unified='+unified,'--',a,b],true);}
function hunks(raw){const lines=raw.split('\n'),out=[];let current=null;for(const line of lines){const m=/^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@(.*)$/.exec(line);if(m){current={old_start:Number(m[1]),old_count:m[2]===undefined?1:Number(m[2]),active_start:Number(m[3]),active_count:m[4]===undefined?1:Number(m[4]),section:m[5].trim(),lines:[]};out.push(current);}else if(current)current.lines.push(line);}return out;}
function lineMap(row,changes){
 const result=[];let a=1,b=1;
 for(const h of changes){const startA=h.old_start+(h.old_count===0?1:0),startB=h.active_start+(h.active_count===0?1:0);if(startA>a){result.push({kind:'unchanged',old_start:a,old_end:startA-1,active_start:b,active_end:startB-1,offset:b-a});}
 result.push({kind:'changed',old_start:h.old_start,old_count:h.old_count,active_start:h.active_start,active_count:h.active_count});a=startA+h.old_count;b=startB+h.active_count;}
 const oldEnd=row.old?.total_lines??0,newEnd=row.active?.total_lines??0;
 if(a<=oldEnd||b<=newEnd)result.push({kind:oldEnd-a===newEnd-b?'unchanged':'terminal_line_shape',old_start:a,old_end:oldEnd,active_start:b,active_end:newEnd,offset:b-a});
 return result;
}
function normalizeIdentity(text,rel){
 let s=text.replace(/CARGO_BIN_EXE_ice\b/g,'CARGO_BIN_EXE_icers').replace(/(?<![A-Za-z0-9_-])ice(?![A-Za-z0-9_-])/g,'icers').replace(/\bICE(?=_|\b)/g,'ICERS');
 if(rel.endsWith('.rs'))s=s.replaceAll('Zyk-Void/ice-rust','zyk-void/ice-rust');
 return s;
}
const [mode,...args]=process.argv.slice(2);
if(mode==='preflight'){
 const rows=inventory(),old=identity(roots.old),active=identity(roots.active),counts={};for(const r of rows)counts[r.status]=(counts[r.status]??0)+1;
 console.log(JSON.stringify({old,active,frozen_rust:frozen.identities.rust,old_source_drift:rows.filter(r=>r.frozen_source_matches===false).map(r=>r.old_path),tracked_old:rows.filter(r=>r.old).length,tracked_active:rows.filter(r=>r.active).length,total_mapped:rows.length,counts,changed_files:rows.filter(r=>r.status!=='identical').map(r=>({old_path:r.old_path,active_path:r.active_path,status:r.status,category:r.category,old_lines:r.old?.total_lines,active_lines:r.active?.total_lines})),commit_changes:commitChanges(),read_only:true}));
}else if(mode==='changes-summary'){
 const rows=inventory().filter(r=>r.status!=='identical').map(row=>{
 if(row.status==='renamed_identical')return {...row,classification:'path-only rename',identity_normalization_equal:true};
 if(row.old?.kind==='text'&&row.active?.kind==='text'){
 const a=content(roots.old,row.old_path).toString('utf8'),b=content(roots.active,row.active_path).toString('utf8'),n=normalizeIdentity(a,row.old_path);
 return {...row,identity_normalization_equal:n===b,identity_plus_whitespace_equal:n.replace(/\s/g,'')===b.replace(/\s/g,''),classification:n===b?'exact identity transformation':'requires hunk review'};
 }return {...row,classification:'requires hunk review'};
 });
 console.log(JSON.stringify({total:rows.length,start:Number(args[0]||0),rows:rows.slice(Number(args[0]||0),Number(args[0]||0)+Number(args[1]||100))}));
}else if(mode==='map'){
 const rows=inventory(),start=Number(args[0]||0),count=Number(args[1]||100);
 console.log(JSON.stringify({total:rows.length,start,rows:rows.slice(start,start+count)}));
}else if(mode==='diff'||mode==='diff-paths'){
 const all=inventory().filter(r=>!['identical','renamed_identical'].includes(r.status)),start=Number(args[0]||0),count=Number(args[1]||5);
 const chosen=mode==='diff-paths'?all.filter(r=>args.includes(r.old_path)||args.includes(r.active_path)):all.slice(start,start+count);
 const rows=chosen.map(row=>{if((row.old?.kind&&row.old.kind!=='text')||(row.active?.kind&&row.active.kind!=='text'))return {...row,diff_skipped:'Non-text or excluded path'};const exact=diff(row,3),zero=diff(row,0),changes=hunks(zero);return {...row,unified_diff:exact,diff_sha256:sha(exact),changes:changes.map(({lines,...h})=>h),line_map:lineMap(row,changes)};});
 const bounded=[];let bytes=200;for(const row of rows){const size=Buffer.byteLength(JSON.stringify(row),'utf8');if(bytes+size>90000)break;bounded.push(row);bytes+=size+1;}console.log(JSON.stringify({total:all.length,start,next:start+bounded.length,requested:rows.length,rows:bounded,byte_budget:90000,first_omitted:bounded.length<rows.length?{path:rows[bounded.length].old_path??rows[bounded.length].active_path,bytes:Buffer.byteLength(JSON.stringify(rows[bounded.length]),'utf8')}:null}));
}else if(mode==='citations'){
 const names=fs.readdirSync(evidence).filter(n=>/^spec-(?:safe-verify|cognee|blackhole|agent|tools|subagents|extensions|observability|settings|providers-auth|session|ai|rpc-remote|cli-packaging|misc|tui)\.json$/.test(n));
 const ai=JSON.parse(fs.readFileSync(path.join(evidence,'spec-ai.json'),'utf8'));if(!ai.entries.some(e=>e.id==='AI-019')&&fs.existsSync(path.join(evidence,'spec-ai-schema-supplement.json')))names.push('spec-ai-schema-supplement.json');
 const selected=new Set(inventory().filter(r=>r.status!=='identical').map(r=>r.old_path).filter(Boolean)),rows=[],sources=[];
 for(const name of names){const raw=fs.readFileSync(path.join(evidence,name),'utf8'),doc=JSON.parse(raw);sources.push({path:name,sha256:sha(raw)});for(const e of doc.entries??[]){for(const [field,value] of Object.entries(e)){if(typeof value!=='string')continue;const re=/(?:rust:)?((?:crates|packages|examples|scripts)\/[A-Za-z0-9_.\/-]+):(\d+)(?:-(\d+))?/g;for(const m of value.matchAll(re)){if(!selected.has(m[1]))continue;rows.push({spec:name,entry_id:e.id,field,path:m[1],start:Number(m[2]),end:Number(m[3]??m[2]),original_reference:m[0]});}}}}
 const start=Number(args[0]||0),count=Number(args[1]||100);console.log(JSON.stringify({sources,total:rows.length,start,rows:rows.slice(start,start+count)}));
}else throw Error('Unknown mode');
