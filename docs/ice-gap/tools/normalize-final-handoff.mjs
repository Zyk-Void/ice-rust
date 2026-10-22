// Normalize final handoff metadata without changing verified behavior.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {fileURLToPath} from 'node:url';
const E=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../evidence');
const area=process.argv[2];
if(!['tui','misc'].includes(area))throw Error('tui or misc');
const read=n=>JSON.parse(fs.readFileSync(path.join(E,n),'utf8'));
const hash=n=>crypto.createHash('sha256').update(fs.readFileSync(path.join(E,n))).digest('hex');
if(read('progress.json').complete||fs.existsSync(path.join(E,area+'.json')))throw Error('Already accepted/completed');
const before={spec:hash('spec-'+area+'.json'),proposal:hash('proposal-'+area+'.json')};
const m=read('proposal-'+area+'.json'),s=read('spec-'+area+'.json');
const report={owner:'/root',captured_at:new Date().toISOString(),area,before,normalized_claims:[],supplemental_ledger:null,source_anchor_additions:[]};
for(const c of m.claims??[]){
 if(!c.first_pass_lines){
  const r=c.report_refs?.find(x=>x.version==='old')??c.report_refs?.[0];
  if(!r)throw Error('Claim lacks report provenance');
  c.first_pass_lines=r.lines;
  c.first_pass_source={path:r.version==='old'?'/home/mewtwo/Zks/ice-rust/ICE_PORTING.md':'/home/mewtwo/Zykairotis/ice-rust/ICE_PORTING.md',sha256:r.version==='old'?'d66e4d33ef0023d228110786f03aeb73c97e72df454b1e481f375b3ac264d7fc':'43463b570bbcaf5d718c35c14e242f0d73afe3f72624cf4fc16edaf25a86627d'};
  c.verdict=c.verdict??c.disposition;
  report.normalized_claims.push(c.id??c.claim);
 }
 if(!Array.isArray(c.evidence))throw Error('Claim lacks source evidence');
}
if(area==='tui'){
 if(before.spec!=='6348be7e858ee5d22c78cfe04f6fb003b00b9f9ed55b66c0271d78995c38254d'||before.proposal!=='9403cd7fd852645553548fb60bb10192b5a38386d66dc3fd24f088221e10de89')throw Error('Released TUI handoff changed');
 const classified=read('tui-supplemental-ledger-classification.json');
 m.supplemental_read_dependencies=m.supplemental_ledger;
 const additions=[
 {path:'packages/tui/native/darwin/README.md',lines:21,entry_ids:['TUI-208'],sha256:'610b5d0ad15af4ac467242bded1786e19812b1d1aed9c6ee1e64d1e31aebe6c7',notes:'Darwin modifier-addon distribution/build guide; documentary guidance only. Full source read and clean frozen Git identity in tui-source-review.json; independent root read1-21. No build/toolchain claim was executed.'},
 {path:'packages/tui/package.json',lines:52,entry_ids:['TUI-208','TUI-260'],sha256:'095cdcc7f4ee8031e6e5a479cd07e802c4976ad5a7b71be2cd47ad3c66401601',notes:'TUI native addon distribution and pinned Markdown parser dependency metadata. Full source read and frozen identity recorded; no dependency was installed.'},
 {path:'packages/tui/README.md',lines:856,entry_ids:['TUI-212','TUI-216','TUI-217'],sha256:'a88f2012a0375f6d6c04338e48c6bf3be6df638cf116c371a1f8b0ad6fa24fc7',notes:'Published TUI/component/renderer/focus/viewport API guide. Full856-line visible read in tui-source-review.json; implementation contracts independently specified by terminal entries. Documentary examples were not executed.'}
 ].map(r=>({...r,area:'tui',status:'mapped'}));
 m.supplemental_ledger=additions;
 report.supplemental_ledger={classification_artifact:'evidence/tui-supplemental-ledger-classification.json',classification_sha256:hash('tui-supplemental-ledger-classification.json'),original_read_attribution_rows:139,retained_as_read_dependencies:139,excluded_test_or_helper_paths:124,already_canonical_paths:12,proper_non_test_supplemental_rows:3,additions};
 const append=(id,text)=>{const e=s.entries.find(e=>e.id===id);if(!e)throw Error('Missing target entry');e.source+='; '+text;report.source_anchor_additions.push({id,text});};
 append('TUI-208','ice:packages/tui/native/darwin/README.md:1-21; ice:packages/tui/package.json:7-21');
 append('TUI-216','ice:packages/tui/README.md:55-79');
 append('TUI-212','ice:packages/tui/README.md:81-95');
 append('TUI-217','ice:packages/tui/README.md:40-49');
}
fs.writeFileSync(path.join(E,'spec-'+area+'.json'),JSON.stringify(s,null,2)+'\n');
fs.writeFileSync(path.join(E,'proposal-'+area+'.json'),JSON.stringify(m,null,2)+'\n');
report.after={spec:hash('spec-'+area+'.json'),proposal:hash('proposal-'+area+'.json')};
fs.writeFileSync(path.join(E,area+'-coordinator-metadata-normalization.json'),JSON.stringify(report,null,2)+'\n');
console.log(JSON.stringify(report,null,2));
