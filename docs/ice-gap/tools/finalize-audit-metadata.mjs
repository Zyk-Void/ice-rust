// Final metadata transitions are gated by the completed source-only validation phase.
import fs from'node:fs';import path from'node:path';import{fileURLToPath}from'node:url';import{execFileSync}from'node:child_process';
const B=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..'),E=path.join(B,'evidence'),read=n=>JSON.parse(fs.readFileSync(path.join(E,n),'utf8'));
const mode=process.argv[2],gate=read('final-validation.json');
if(!['original','active-complete'].includes(mode))throw Error('original|active-complete');
if(!gate.passed||gate.phase!==(mode==='original'?'original':'active'))throw Error('Matching final gate must pass first');
if(path.resolve(gate.identities.rust.root)!==path.resolve(B,'../..'))throw Error('Wrong checkout');
const q=read('lane-exports-rpc-ai-auth-header-metadata-qa.json'),chosen=q.replacements.filter(x=>mode==='active-complete'||['header-1','header-5','header-7','header-8'].includes(x.id));
const priorName='final-metadata-original.json',prior=fs.existsSync(path.join(E,priorName))?read(priorName):null;
const changed=new Map(),records=[];
for(const r of chosen){
 const name='spec-'+r.area+'.json',s=changed.get(name)??read(name);
 if(mode==='active-complete'&&prior?.changes.some(x=>x.id===r.id)&&r.id!=='header-7')continue;
 if(r.mode==='replace-exact-root-field-value'){
  if(s[r.field]!==r.old_value)throw Error('Stale '+r.id);records.push({id:r.id,name,field:r.field,before:s[r.field],after:r.new_value});s[r.field]=r.new_value;
 }else{
  let old=r.old_text,next=r.new_text;
  if(r.id==='header-7'){
   const intermediate='TypeScript behavior is based on the frozen dirty-worktree baseline; Rust comparisons target ff9270f. This area is published. Overall completion awaits the active-checkout relocation.';
   if(mode==='original')next=intermediate;else if(prior)old=intermediate;
  }
  if(!s[r.field]?.includes(old))throw Error('Stale '+r.id);
  const after=s[r.field].replace(old,next);records.push({id:r.id,name,field:r.field,before:s[r.field],after});s[r.field]=after;
 }
 changed.set(name,s);
}
const areas=read('progress.json').areas.map(x=>x.area);
if(mode==='active-complete')for(const a of areas){
 const name='spec-'+a+'.json',s=changed.get(name)??read(name);s.complete=true;s.draft_complete=true;s.accepted_phase='active-source-verified';changed.set(name,s);
 const name2=a+'.json',m=read(name2);m.complete=true;m.draft_complete=true;m.published=true;m.accepted_phase='active-source-verified';changed.set(name2,m);
}
for(const[n,v]of changed)fs.writeFileSync(path.join(E,n),JSON.stringify(v,null,2)+'\n');
const renderer=fs.existsSync(path.join(B,'render-area.mjs'))?path.join(B,'render-area.mjs'):path.join(B,'tools/render-area.mjs');
for(const a of new Set([...changed.keys()].filter(x=>x.startsWith('spec-')).map(x=>x.slice(5,-5))))execFileSync(process.execPath,[renderer,a],{cwd:path.resolve(B,'../..'),stdio:'pipe'});
fs.writeFileSync(path.join(E,'final-metadata-'+mode+'.json'),JSON.stringify({captured_at:new Date().toISOString(),mode,gate:gate.captured_at,changes:records,completed_areas:mode==='active-complete'?areas:[]},null,2)+'\n');
console.log(JSON.stringify({mode,changed_files:changed.size,metadata_replacements:records.length},null,2));
