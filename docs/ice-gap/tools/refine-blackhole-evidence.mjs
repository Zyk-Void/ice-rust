import { auditHelperBase, auditHelperTools } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
assertAuditOpen();
// Correct exact exported names, setting labels and citation lines after source review.
import fs from 'node:fs';
import path from 'node:path';
const base=auditHelperBase;
const specFile=path.join(base,'evidence/spec-blackhole.json');
const spec=JSON.parse(fs.readFileSync(specFile,'utf8'));
const corrections=[['DEFAULT_CONFIG','DEFAULTS'],['version0.4.3','version 0.4.3'],['blackhole0.4.3','blackhole 0.4.3'],['k0valik) and','k0valik) and'],['(2026 k0valik)','(2026 k0valik)'],['between1 and99','between 1 and 99'],['finite1..99','finite 1..99'],['number1..99','number 1..99'],['1..99%','1..99%'],['labels1%','labels 1%'],['with mode0700','with mode 0700'],['with mode0600','with mode 0600'],['the file0600','the file 0600'],['to85','to 85'],['becomes0','becomes 0'],['selects85','selects 85'],['minimum1','minimum 1'],['yield0','yield 0'],['defaults200','defaults 200'],['clipSentence150','clipSentence 150'],['word budget256','word budget 256'],['budget256','budget 256'],['budget200','budget 200'],['up to10','up to 10'],['most10','most 10'],['to120','to 120'],['clipped60','clipped 60'],['line clipped150','line clipped 150'],['last20','last 20'],['first5','first 5'],['latest5','latest 5'],['length>=15','length >= 15'],['mode exactlybuild','mode exactly build'],['defaultunknown','default unknown'],['objective400','objective 400'],['clipped400','clipped 400'],['text240','text 240'],['files240','files 240'],['title160','title 160'],['last12','last 12'],['capped24000','capped 24000'],['requested300','requested 300'],['maxTokens1200','maxTokens 1200'],['additional45','additional 45'],['nominal4000','nominal 4000'],['true4000','true 4000'],['cap24','cap 24'],['expiry15min','expiry 15 min'],['text budget6000','text budget 6000'],['a6000','a 6000'],['within4000','within 4000'],['beyond300','beyond 300'],['after300','after 300'],['to300','to 300'],['limit6000','limit 6000'],['at6000','at 6000'],['after3000','after 3000'],['min3000','min 3000'],['min1500','min 1500'],['clipped512','clipped 512'],['defaultfalse','default false'],['default0','default 0'],['requested=false','requested=false'],['the20-block','the 20-block'],['description80','description 80'],['conclusion160','conclusion 160']];
for(const e of spec.entries)for(const field of ['source','what','mechanism','port','priority_reason'])if(typeof e[field]==='string')for(const [a,b]of corrections)e[field]=e[field].replaceAll(a,b);
fs.writeFileSync(specFile,JSON.stringify(spec,null,2)+'\n');
let text=fs.readFileSync(path.join(auditHelperTools,'finalize-blackhole.mjs'),'utf8');
text=text.replace("add('settings','blackhole.config.'+name", "add('settings',name");
text=text.replace("add('settings','blackhole.ui.'+name", "add('settings','blackhole.'+name");
text=text.replace("['compactAfterPercent',16,2]","['compactAfterPercent',15,2]").replace("['memory',17,2]","['memory',16,2]");
text=text.replace("['ice-subagent-v2-child','src/core/subagents.ts',10,34]","['ice-subagent-v2-child','src/core/subagents.ts',15,34]");
const insertion=`
// Resolve concrete declaration/call lines rather than retain guessed comment lines.
function exactLine(p,needle){const a=fs.readFileSync(path.join(root,prefix,p),'utf8').split('\\n');const i=a.findIndex(l=>l.includes(needle));if(i<0)throw Error('No source occurrence: '+needle);return i+1;}
for(const s of surfaces){
 if(s.name==='ice-cognee-recall')s.ice_ref=prefix+'src/core/tail.ts:'+exactLine('src/core/tail.ts','"ice-cognee-recall"');
 if(s.name==='compaction.firstKeptEntryId=__blackhole_minimal_tail__')s.ice_ref=prefix+'src/core/tail.ts:'+exactLine('src/core/tail.ts','"__blackhole_minimal_tail__"');
 if(s.name==='Blackhole compactAfterTurn(mode)')s.ice_ref=prefix+'src/om/compaction-trigger.ts:'+exactLine('src/om/compaction-trigger.ts','ctx.compactAfterTurn(');
 if(s.category==='settings')s.notes=s.name.startsWith('blackhole.')?'UI namespace blackhole followed by exact item ID; not a persisted dotted JSON key.':'Exact top-level JSON key in ice-blackhole-config.json.';
}
`;
if(!text.includes('Resolve concrete declaration/call lines'))text=text.replace('const resolved=new Set',insertion+'\nconst resolved=new Set');
fs.writeFileSync(path.join(auditHelperTools,'finalize-blackhole.mjs'),text);
console.log('Corrected exported DEFAULTS, spacing and exact Blackhole surface citations.');
