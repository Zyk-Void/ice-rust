// Read-only preparation and syntax validation shared by the reviewed helper migration.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
export const PREFIX='lane-exports-rpc-ai-auth';
export const PLAN_NAME='helper-migration-plan.json';
export const QA_NAME=PREFIX+'-migration-qa.json';
export const ACTIVE_AUDIT='/home/mewtwo/Zykairotis/ice-rust/docs/ice-gap';
export const OLD_AUDIT='/home/mewtwo/Zks/ice-rust/docs/ice-gap';
export const sha=bytes=>crypto.createHash('sha256').update(bytes).digest('hex');
export function safe(base,rel,allowMissing=false){
 if(typeof rel!=='string'||!rel||path.isAbsolute(rel)||rel.split(/[\\/]/).some(x=>!x||x==='.'||x==='..'))throw Error('Invalid audit-relative path: '+rel);
 const full=path.resolve(base,rel);
 if(!full.startsWith(base+path.sep))throw Error('Outside audit boundary: '+rel);
 let current=base;
 for(const part of rel.split('/')){
  current=path.join(current,part);
  try{if(fs.lstatSync(current).isSymbolicLink())throw Error('Symlink is not a migration input/target: '+rel);}
  catch(error){if(allowMissing&&error.code==='ENOENT')break;throw error;}
 }
 return full;
}
export function walkFiles(base,rel){
 const dir=safe(base,rel);
 const rows=[];
 for(const entry of fs.readdirSync(dir,{withFileTypes:true})){
  const name=rel+'/'+entry.name;
  if(entry.isSymbolicLink())throw Error('Symlink in audited helper tree: '+name);
  if(entry.isDirectory())rows.push(...walkFiles(base,name));
  else if(entry.isFile())rows.push(name);
  else throw Error('Nonregular audit file: '+name);
 }
 return rows.sort();
}
export function inventory(base){
 const root=fs.readdirSync(base,{withFileTypes:true}).filter(e=>e.isFile()&&/\.(mjs|py)$/.test(e.name)).map(e=>e.name);
 for(const e of fs.readdirSync(base,{withFileTypes:true}))if(e.isSymbolicLink()&&/\.(mjs|py)$/.test(e.name))throw Error('Symlink helper: '+e.name);
 const evidence=walkFiles(base,'evidence').filter(p=>/\.(mjs|py)$/.test(p));
 const tools=walkFiles(base,'tools').filter(p=>/\.(mjs|py)$/.test(p));
 return {loose:[...root,...evidence].sort(),tools,all:[...root,...evidence,...tools].sort()};
}
export function transform(text,row){
 let result=text;
 for(const edit of row.patches){
  if(typeof edit.old_text!=='string'||!edit.old_text||typeof edit.new_text!=='string'||edit.expected_occurrences!==1)throw Error('Invalid exact patch '+row.source);
  const count=result.split(edit.old_text).length-1;
  if(count!==1)throw Error('Stale exact patch '+row.source+':'+edit.source_line+' ('+count+' matches)');
  result=result.replace(edit.old_text,()=>edit.new_text);
 }
 if(row.prepend){
  if(result.startsWith('#!')){const at=result.indexOf('\n');if(at<0)throw Error('Unterminated shebang');result=result.slice(0,at+1)+row.prepend+result.slice(at+1);}
  else result=row.prepend+result;
 }
 return result;
}
export function readPreparation(base){
 if(fs.realpathSync(base)!==base)throw Error('Audit root must be its exact real path');
 const planPath=safe(base,'evidence/'+PLAN_NAME),planBytes=fs.readFileSync(planPath),plan=JSON.parse(planBytes);
 if(plan.schema_version!==1||plan.status!=='ready-for-active-copy'||plan.active_audit_root!==ACTIVE_AUDIT||plan.old_audit_root!==OLD_AUDIT||plan.source_only!==true||plan.application_or_test_execution!==false)throw Error('Unexpected migration plan boundary/schema');
 const seenSource=new Set(),seenTarget=new Set(),scripts=[];
 for(const row of plan.scripts){
  if(!/^(?:[^/]+|(?:evidence|tools)\/[^/]+)\.(?:mjs|py)$/.test(row.source)||!/^tools\/[^/]+\.(?:mjs|py)$/.test(row.destination))throw Error('Unsupported script move: '+row.source);
  if(seenSource.has(row.source)||seenTarget.has(row.destination))throw Error('Duplicate script source/destination');
  seenSource.add(row.source);seenTarget.add(row.destination);
  if(row.move!==(row.source!==row.destination))throw Error('Invalid move flag');
  const source=safe(base,row.source),destination=safe(base,row.destination,true),raw=fs.readFileSync(source);
  if(sha(raw)!==row.before_sha256)throw Error('Reviewed helper changed: '+row.source);
  if(row.move&&fs.existsSync(destination))throw Error('Refusing destination collision: '+row.destination);
  const text=transform(raw.toString('utf8'),row),after=Buffer.from(text,'utf8');
  if(row.after_sha256!==sha(after)||row.after_bytes!==after.length)throw Error('Planned transformed bytes disagree: '+row.source);
  scripts.push({row,raw,after,after_sha256:sha(after),changed:!raw.equals(after),mode:fs.statSync(source).mode&0o777});
 }
 const support=[],seenSupport=new Set();
 for(const record of plan.support_files){
  if(!/^tools\/lane-exports-rpc-ai-auth-[a-z-]+\.mjs$/.test(record.path)||seenSource.has(record.path)||seenTarget.has(record.path)||seenSupport.has(record.path))throw Error('Invalid or duplicate support helper '+record.path);
  seenSupport.add(record.path);
  const raw=fs.readFileSync(safe(base,record.path));
  if(sha(raw)!==record.sha256)throw Error('Prepared support helper changed: '+record.path);
  support.push({...record,raw});
 }
 // All dependency forms in these hash-bound inputs were manually inspected.
 // Current local dependencies are single-line static imports; this check covers that reviewed form.
 const available=new Set([...scripts.map(s=>s.row.destination),...support.map(s=>s.path)]);
 for(const script of [...scripts.map(s=>({path:s.row.destination,text:s.after.toString('utf8')})),...support.map(s=>({path:s.path,text:s.raw.toString('utf8')}))]){
  const imports=/(?:^|\n)\s*import\s+[^;\n]*?\bfrom\s*(['"])(\.[^'"]+)\1/g;
  for(const match of script.text.matchAll(imports)){
   const target=path.posix.normalize(path.posix.join(path.posix.dirname(script.path),match[2]));
   if(!available.has(target))throw Error('Unresolved relocated relative import '+script.path+' -> '+target);
  }
 }
 const actual=inventory(base),known=new Set([...seenSource,...support.map(s=>s.path)]);
 const unknown=actual.all.filter(p=>!known.has(p));
 if(unknown.length)throw Error('Unreviewed helper inventory additions: '+unknown.join(', '));
 const expectedLoose=plan.scripts.filter(s=>s.move).map(s=>s.source).sort();
 if(JSON.stringify(actual.loose)!==JSON.stringify(expectedLoose))throw Error('Root/evidence helper inventory differs from the explicit move map');
 const review=plan.independent_review;
 if(review.source!=='independent-review'||review.destination!=='evidence/independent-review')throw Error('Unexpected independent-review move');
 if(fs.existsSync(safe(base,review.destination,true)))throw Error('Independent-review destination already exists');
 const reviewFiles=walkFiles(base,review.source);
 if(JSON.stringify(reviewFiles)!==JSON.stringify(review.files.map(r=>r.source).sort()))throw Error('Independent-review contents changed');
 for(const row of review.files){
  if(row.destination!=='evidence/'+row.source||sha(fs.readFileSync(safe(base,row.source)))!==row.sha256)throw Error('Independent-review input changed: '+row.source);
 }
 return {plan,plan_sha256:sha(planBytes),scripts,support,inventory:actual};
}
export function syntaxCheck(name,text){
 const python=name.endsWith('.py');
 const args=python?['-I','-c','import ast,sys; ast.parse(sys.stdin.read(), filename=sys.argv[1])',name]:['--check','--input-type=module'];
 const result=spawnSync(python?'/usr/bin/python3':process.execPath,args,{input:text,encoding:'utf8',timeout:15000,maxBuffer:1024*1024});
 if(python)assertNoPythonResidue(path.dirname(path.dirname(fileURLToPath(import.meta.url))));
 return {path:name,method:python?'/usr/bin/python3 -I ast.parse from stdin':'Node --check --input-type=module from stdin',passed:result.status===0&&!result.error,status:result.status,error:result.error?.message??null,stderr:result.stderr?.trim()??''};
}
export function assertNoPythonResidue(base){
 const found=[];
 function visit(dir){for(const e of fs.readdirSync(dir,{withFileTypes:true})){if(e.name==='__pycache__'||e.name==='.venv')found.push(path.relative(base,path.join(dir,e.name)));else if(e.isDirectory())visit(path.join(dir,e.name));}}
 visit(base);
 const repo=path.resolve(base,'../..');
 for(const name of ['.venv','__pycache__'])if(fs.existsSync(path.join(repo,name)))found.push(path.relative(base,path.join(repo,name)));
 if(found.length)throw Error('Python/tool residue in audit tree: '+found.join(', '));
 return {checked_root:base,forbidden_names:['.venv','__pycache__'],found:[]};
}
export function syntaxPreparation(prepared){
 const results=[...prepared.scripts.map(s=>syntaxCheck(s.row.destination,s.after)),...prepared.support.map(s=>syntaxCheck(s.path,s.raw))];
 assertNoPythonResidue(prepared.plan.old_audit_root);
 return results;
}
