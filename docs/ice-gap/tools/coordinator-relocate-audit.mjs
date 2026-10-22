// Copy a completed original audit to its explicitly authorized active-checkout location.
import fs from'node:fs';import path from'node:path';import crypto from'node:crypto';import{execFileSync}from'node:child_process';import{fileURLToPath}from'node:url';
const source='/home/mewtwo/Zks/ice-rust/docs/ice-gap',destination='/home/mewtwo/Zykairotis/ice-rust/docs/ice-gap';
const E=path.join(source,'evidence'),mode=process.argv[2]||'preview',sha=x=>crypto.createHash('sha256').update(x).digest('hex');
if(!['preview','apply'].includes(mode))throw Error('preview|apply');
if(path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..')!==source)throw Error('Run only from the completed original audit');
const gate=JSON.parse(fs.readFileSync(path.join(E,'original-checkout-validation.json'),'utf8')),m=JSON.parse(fs.readFileSync(path.join(E,'active-checkout-source-map.json'),'utf8'));
if(!gate.passed||gate.phase!=='original'||!gate.source_only||gate.application_or_test_execution)throw Error('Passing original gate required');
const git=(root,...args)=>execFileSync('git',['--no-optional-locks','-C',root,...args],{encoding:'utf8',stdio:['ignore','pipe','pipe']}).trim();
const active=m.identities.active;
if(git(active.root,'rev-parse','HEAD')!==active.head||git(active.root,'branch','--show-current')!=='main'||git(active.root,'remote','get-url','origin')!==active.origin)throw Error('Active identity drift');
if(git(active.root,'status','--porcelain=v1','--untracked-files=normal'))throw Error('Active checkout is not clean before relocation');
function inventory(root,dir=''){const out=[];for(const e of fs.readdirSync(path.join(root,dir),{withFileTypes:true})){const rel=path.join(dir,e.name);if(e.isSymbolicLink())throw Error('Symlink in audit '+rel);if(e.isDirectory()){if(['.venv','node_modules','__pycache__'].includes(e.name))throw Error('Tool residue '+rel);out.push(...inventory(root,rel));}else if(e.isFile()){const raw=fs.readFileSync(path.join(root,rel));out.push({path:rel,bytes:raw.length,sha256:sha(raw)});}else throw Error('Unsupported audit entry '+rel);}return out.sort((a,b)=>a.path.localeCompare(b.path));}
const files=inventory(source),exists=fs.existsSync(destination);
if(exists)throw Error('Destination already exists; preserve it and inspect rather than overwrite');
const summary={mode,source,destination,files:files.length,bytes:files.reduce((n,f)=>n+f.bytes,0),active_head:active.head,original_gate:gate.captured_at,original_contracts_complete:true};
if(mode==='apply'){
 fs.mkdirSync(path.dirname(destination),{recursive:true});execFileSync('/bin/cp',['-a',source,destination],{stdio:'pipe'});
 const originalAfter=inventory(source),copied=inventory(destination);
 if(JSON.stringify(files)!==JSON.stringify(originalAfter)||JSON.stringify(files)!==JSON.stringify(copied))throw Error('Copy integrity mismatch; audit copies retained for investigation');
 const report={...summary,captured_at:new Date().toISOString(),original_left_intact:true,exact_copy_verified:true,source_only:true,original_gate_sha256:sha(fs.readFileSync(path.join(E,'original-checkout-validation.json'))),files};
 fs.writeFileSync(path.join(destination,'evidence/active-relocation-copy.json'),JSON.stringify(report,null,2)+'\n');
}
console.log(JSON.stringify(summary,null,2));
