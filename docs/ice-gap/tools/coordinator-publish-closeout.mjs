// Coordinator closeout publication. Only audit documents are writable.
import fs from 'node:fs'; import path from 'node:path'; import crypto from 'node:crypto';
import {fileURLToPath} from 'node:url'; import {execFileSync} from 'node:child_process';
const B=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..'),E=path.join(B,'evidence');
const read=n=>JSON.parse(fs.readFileSync(path.join(E,n),'utf8'));
const sha=n=>crypto.createHash('sha256').update(fs.readFileSync(path.join(E,n))).digest('hex');
const area=process.argv[2];if(!['misc','tui','ai'].includes(area))throw Error('Expected misc, tui or ai');
if(read('progress.json').complete)throw Error('Published audit protected');
const run=(p,a=[])=>execFileSync(process.execPath,[path.join(B,p),...a],{cwd:path.resolve(B,'../..'),encoding:'utf8',maxBuffer:32*1024*1024});
if(area==='misc'){
 if(fs.existsSync(path.join(E,'misc.json')))throw Error('MISC already published');
 const q=read('lane-final-areas-misc-qa.json');
 if(q.spec_sha256!==sha('spec-misc.json')||q.entries!==43||q.required_source_gaps.length||q.identity_conflicts.length||q.candidate_accounting.pending.length||q.candidate_accounting.overlap.length)throw Error('MISC handoff QA mismatch');
 fs.copyFileSync(path.join(E,'lane-final-areas-proposal-misc.json'),path.join(E,'proposal-misc.json'));
 run('tools/normalize-final-handoff.mjs',['misc']);
 const v=JSON.parse(run('tools/verify-area-evidence.mjs',['misc']));if(v.problems.length)throw Error('MISC validation failed');
 console.log(run('tools/accept-final-area.mjs',['misc',sha('spec-misc.json'),sha('proposal-misc.json')]));
}else if(area==='tui'){
 if(!fs.existsSync(path.join(E,'misc.json'))||fs.existsSync(path.join(E,'tui.json')))throw Error('Publication order');
 const q=read('lane-final-areas-tui-qa.json'),s=read('spec-tui.json');
 if(s.entries.length!==151||new Set(s.entries.map(e=>e.id)).size!==151||q.entries!==151)throw Error('TUI count');
 const terminal=read('spec-tui-terminal.json').entries;
 if(!terminal.every(e=>s.entries.some(x=>x.id===e.id)))throw Error('Terminal slice absent');
 const v=JSON.parse(run('tools/verify-area-evidence.mjs',['tui']));if(v.problems.length)throw Error('TUI validation failed');
 console.log(run('tools/accept-final-area.mjs',['tui',sha('spec-tui.json'),sha('proposal-tui.json')]));
}else{
 console.log(run('tools/integrate-ai-schema-supplement.mjs'));
 console.log(run('tools/publish-area.mjs',['ai']));
}
const p=read('progress.json');
fs.appendFileSync(path.join(E,'RESUME.md'),'\n## Publication checkpoint '+new Date().toISOString()+'\n\n'+area+' published by the coordinator. Current total '+p.entry_count+' entries, '+p.ledger_rows+' ledger rows ('+p.pending+' pending). Surface pending '+Object.values(p.surface_counts).reduce((n,x)=>n+x.pending,0)+'. TUI terminal entries are already included in the 151 count; do not merge them twice. SESSION-039/040 remain reserved but unused after native counterpart confirmation. Next: '+(area==='misc'?'publish TUI':area==='tui'?'integrate AI-019':'merge export corrections and reconcile dependencies, claims, surfaces and canonical read evidence')+'. Global completion remains false.\n');
console.log(JSON.stringify({published:area,entry_count:p.entry_count,ledger:p.ledger_rows,pending_ledger:p.pending,surface_pending:Object.values(p.surface_counts).reduce((n,x)=>n+x.pending,0)}));
