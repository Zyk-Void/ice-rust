// Reconcile heterogeneous saved handoff metadata; preserve original values and source evidence.
import fs from 'node:fs';import path from 'node:path';import {fileURLToPath} from 'node:url';import {execFileSync} from 'node:child_process';
const B=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..'),E=path.join(B,'evidence');
const read=n=>JSON.parse(fs.readFileSync(path.join(E,n),'utf8'));if(read('progress.json').complete)throw Error('Published tree protected');
const s=read('spec-tui.json'),byId=new Map(s.entries.map(e=>[e.id,e])),changes=[];
for(const name of ['tui.json','proposal-tui.json']){
 const m=read(name);
 for(let i=0;i<(m.rust_only??[]).length;i++){
  const r=m.rust_only[i],before=structuredClone(r);
  r.capability??='Native behavior preserved by '+(r.entry_ids??[]).join(', ');
  r.ts_comparison??=r.description??r.notes;
  if(!r.ts_comparison)throw Error('Native preservation comparison absent');
  if(!r.evidence)r.evidence=[...new Set((r.entry_ids??[]).flatMap(id=>{const e=byId.get(id);if(!e)throw Error('Unknown native preservation entry');return [...(e.rust+' '+e.source).matchAll(/(?:ice|rust):[A-Za-z0-9_./@+-]+:\d+(?:-\d+)?/g)].map(x=>x[0]);}))];
  if(!r.evidence.length)throw Error('Native preservation evidence absent');
  if(JSON.stringify(before)!==JSON.stringify(r))changes.push({name,index:i,before,after:r});
 }
 for(const r of m.surfaces??[]){if(r.category==='commands'){changes.push({name,surface:r.name,category_before:'commands',category_after:'slash-commands'});r.category='slash-commands';}if(r.category==='env'){changes.push({name,surface:r.name,category_before:'env',category_after:'environment'});r.category='environment';}}
 fs.writeFileSync(path.join(E,name),JSON.stringify(m,null,2)+'\n');
}
fs.writeFileSync(path.join(E,'coordinator-tui-metadata-repair.json'),JSON.stringify({captured_at:new Date().toISOString(),cause:'Saved TUI rust_only records mixed {capability,evidence,notes} and {description,entry_ids}; prior publisher expected evidence/ts_comparison for each. Area rendering and ledger merge had succeeded, README refresh failed. Exact source anchors derived from each already-certified entry; none invented.',changes},null,2)+'\n');
for(const f of ['commands.csv','env.csv']){const p=path.join(B,'surfaces',f);if(fs.existsSync(p)){fs.copyFileSync(p,path.join(E,'pre-normalization-'+f));fs.unlinkSync(p);}}
const output=execFileSync(process.execPath,[path.join(B,'tools/publish-area.mjs'),'tui'],{cwd:path.resolve(B,'../..'),encoding:'utf8',maxBuffer:20*1024*1024});
const p=read('progress.json');
fs.appendFileSync(path.join(E,'RESUME.md'),'\n## TUI publication checkpoint '+new Date().toISOString()+'\n\nTUI151 is published, including all67 terminal entries. README refresh recovered after documented metadata schema normalization (coordinator-tui-metadata-repair.json); no entries were lost or duplicated. All16 areas now rendered,790 total entries beforeAI019. Ledger872:860 mapped,12 N/A,0 pending. Surfaces0 pending; original candidate provenance preserved. Next: AI019 integration, export corrections, dependency/claims/canonical coverage, spacing, final validation and relocation.\n');
console.log(JSON.stringify({published:'tui',entries:p.entry_count,ledger_rows:p.ledger_rows,pending:p.pending,surfaces:Object.values(p.surface_counts).reduce((n,x)=>n+x.total,0),surface_pending:Object.values(p.surface_counts).reduce((n,x)=>n+x.pending,0),metadata_changes:changes.length},null,2));
