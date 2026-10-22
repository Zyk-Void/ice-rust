// Repair accepted publication metadata and current matrix citation syntax; no behavior edits.
import fs from'node:fs';import path from'node:path';import{fileURLToPath}from'node:url';import{execFileSync}from'node:child_process';
const B=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..'),E=path.join(B,'evidence'),read=n=>JSON.parse(fs.readFileSync(path.join(E,n),'utf8'));
if(read('progress.json').complete)throw Error('Published complete tree is protected');
const m=read('spec-misc.json');if(m.title||m.introduction)throw Error('Misc metadata already set');
m.title='Miscellaneous utilities and integrations';
m.introduction='Source-only specification of shared utilities, evaluation workflows and Claude SDK integration. The 43 accepted entries retain exact TypeScript contracts, native counterparts, source/test evidence and explicit external-system limits. Application code, tests, evaluation runners and SDK integrations were read without executing them.';
fs.writeFileSync(path.join(E,'spec-misc.json'),JSON.stringify(m,null,2)+'\n');
const report={captured_at:new Date().toISOString(),misc_metadata:{title:m.title,introduction:m.introduction},matrix_changes:[]};
const f=read('qa-first-pass-coverage.json');
const pattern=/\b(ice|rust):([A-Za-z0-9_./@+\-]+):(\d+(?:[-–]\d+)?)(?:((?:,\s*\d+(?:[-–]\d+)?)+))/g;
for(const c of f.claims)for(const key of ['evidence','finding'])if(c[key]){
 const convert=(s)=>s.replace(pattern,(old,side,p,first,rest)=>{const next=[first,...rest.split(',').map(x=>x.trim()).filter(Boolean)].map(r=>side+':'+p+':'+r).join('; ');report.matrix_changes.push({id:c.id,key,before:old,after:next});return next;});
 c[key]=Array.isArray(c[key])?c[key].map(convert):convert(c[key]);
}
fs.writeFileSync(path.join(E,'qa-first-pass-coverage.json'),JSON.stringify(f,null,2)+'\n');
const renderer=fs.existsSync(path.join(B,'render-area.mjs'))?path.join(B,'render-area.mjs'):path.join(B,'tools/render-area.mjs');
execFileSync(process.execPath,[renderer,'misc'],{cwd:path.resolve(B,'../..'),stdio:'pipe'});
fs.writeFileSync(path.join(E,'original-publication-metadata-repair.json'),JSON.stringify(report,null,2)+'\n');
console.log(JSON.stringify({misc_title:m.title,matrix_citation_expansions:report.matrix_changes.length},null,2));
