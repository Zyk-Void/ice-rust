// Read-only inspection of saved relocation evidence; no source execution.
import fs from 'node:fs';import path from 'node:path';import{fileURLToPath}from'node:url';
const E=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../evidence'),d=JSON.parse(fs.readFileSync(path.join(E,'active-checkout-diff-hunks-complete.json'),'utf8'));
const [mode='list',start='0',count='5']=process.argv.slice(2);
const isAI=p=>/^crates\/ice-(?:ai|agent|tools)\//.test(p),isHarness=p=>/^crates\/ice-(?:harness|tui)\//.test(p);
let rows=d.rows.filter(r=>!isAI(r.old_path)&&!isHarness(r.old_path));
if(mode==='list'){console.log(JSON.stringify(rows.map((r,i)=>({i,path:r.old_path,active:r.active_path,hunks:r.changes.length,chars:r.unified_diff?.length??0})),null,2));}
else if(mode==='root'){for(const[i,r]of rows.entries())if(i>=+start&&i<+start+ +count)console.log(JSON.stringify({i,path:r.old_path,active:r.active_path,sha256:r.old.sha256,active_sha256:r.active.sha256,changes:r.changes,unified_diff:r.unified_diff},null,2));}
else if(mode==='metadata'){const m=JSON.parse(fs.readFileSync(path.join(E,'active-checkout-source-map.json'),'utf8'));console.log(JSON.stringify({renames:m.rows.filter(r=>r.old_path&&r.active_path!==r.old_path).map(r=>({old:r.old_path,active:r.active_path,status:r.status})),root_files:rows.length,root_hunks:rows.reduce((n,r)=>n+r.changes.length,0)},null,2));}
else throw Error('list|root|metadata');
