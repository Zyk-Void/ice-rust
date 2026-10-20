// Derive a citation-only helper from the reviewed closeout normalizer; audit documents only.
import fs from 'node:fs'; import path from 'node:path'; import {fileURLToPath} from 'node:url';
const T=path.dirname(fileURLToPath(import.meta.url)), B=path.resolve(T,'..'), E=path.join(B,'evidence');
if(JSON.parse(fs.readFileSync(path.join(E,'progress.json'),'utf8')).complete)throw Error('Completed audit protected');
let s=fs.readFileSync(path.join(T,'reconcile-readability.mjs'),'utf8');
const line="   if(['what','mechanism','rust','port','priority_reason','dependency_text'].includes(k))next=prose(next,e.id,k);";
if(!s.includes(line))throw Error('Unexpected normalizer version');
s=s.replace(line,'   // Spacing is handled independently by fix-spacing.mjs with preservation proofs.');
s=s.replace("'readability-final-reconciliation.json'","'citation-normalization.json'");
s=s.replace("console.log(JSON.stringify({...report,fields:undefined,inputs:undefined,changes:report.changes.slice(0,40)},null,2));","fs.writeFileSync(path.join(base,'evidence','citation-normalization-'+mode+'.json'),JSON.stringify(report,null,2)+'\\n');\nconsole.log(JSON.stringify({...report,fields:undefined,inputs:undefined,changes:report.changes.slice(0,10)},null,2));");
fs.writeFileSync(path.join(T,'normalize-citations.mjs'),s);
fs.appendFileSync(path.join(E,'RESUME.md'),'\n\n## Cross-cutting reconciliation '+new Date().toISOString()+'\n\n| Lane | State |\n|---|---|\n| L1/L2/L2b | All 16 areas published, 791 entries |\n| L3/L4 | 704 of 1,654 dispositions merged; SUB, RPC, TUI/MISC in source review |\n| L5 | 872 ledger rows and 3,975 surfaces; zero pending |\n| L6 | 228 dependency requests resolved; 2,830 prerequisite, 92 consumer, 131 shared-contract edges; one external producer explicitly unverified |\n| L7 | All 160 first-pass claims dispositioned, §11a covered; no unresolved follow-ups. RUST-066/067 upstream provenance reused and RUST-068 reviewed provenance applied. |\n| L8/final | Citation-only normalizer prepared; final spacing waits for export merges |\n\nReserved IDs: SESSION-039/040 remain unused. No new gaps inferred from the two existing session counterparts. Next: merge remaining exports, install canonical read coverage, normalize citations/spacing, build index/graph, run original gate, then relocate.\n');
console.log(JSON.stringify({created:'tools/normalize-citations.mjs',spacing_separate:true}));
