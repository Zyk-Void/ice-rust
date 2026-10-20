import { auditHelperBase } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
assertAuditOpen();
// Direct audit authoring helpers. Never executes repository modules or model calls.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {createRequire} from 'node:module';
export const out=auditHelperBase;
export const root='/home/mewtwo/Zks/ice';
const ts=createRequire(path.join(root,'package.json'))('typescript');
export const p={loop:'packages/agent/src/agent-loop.ts',agent:'packages/agent/src/agent.ts',types:'packages/agent/src/types.ts',stream:'packages/agent/src/stream-fn.ts',proxy:'packages/agent/src/proxy.ts',harness:'packages/agent/src/harness/agent-harness.ts',env:'packages/agent/src/harness/env/nodejs.ts',messages:'packages/agent/src/harness/messages.ts',templates:'packages/agent/src/harness/prompt-templates.ts',skills:'packages/agent/src/harness/skills.ts',system:'packages/agent/src/harness/system-prompt.ts',htypes:'packages/agent/src/harness/types.ts',result:'packages/agent/src/harness/result.ts'};
export const S=(key,a,b)=>'ice:'+(p[key]??key)+':'+a+'-'+b;
export const R=(file,a,b)=>'rust:crates/'+file+':'+a+'-'+b;
export function quote(key,a,b,lang='ts'){
 const file=p[key]??key;const text=fs.readFileSync(path.join(root,file),'utf8').split('\n');
 if(a<1||b>text.length)throw Error('Invalid quote range '+file+':'+a+'-'+b);
 return '```'+lang+'\n'+text.slice(a-1,b).join('\n')+'\n```\nSource: '+S(key,a,b)+'.';
}
const tests=new Map();
export function test(file,needle){
 const f='packages/agent/test/'+file;
 if(!tests.has(f)){
  const text=fs.readFileSync(path.join(root,f),'utf8');const sf=ts.createSourceFile(f,text,ts.ScriptTarget.Latest,true);const calls=[];
  function walk(n){if(ts.isCallExpression(n)&&/^(it|test)(\.[A-Za-z]+)?$/.test(n.expression.getText(sf))&&n.arguments[0]&&ts.isStringLiteral(n.arguments[0]))calls.push({name:n.arguments[0].text,start:sf.getLineAndCharacterOfPosition(n.getStart(sf)).line+1,end:sf.getLineAndCharacterOfPosition(n.getEnd()).line+1});ts.forEachChild(n,walk);}walk(sf);tests.set(f,calls);
 }
 const matches=tests.get(f).filter(t=>t.name.includes(needle));if(matches.length!==1)throw Error('Ambiguous test '+file+' '+needle+' '+matches.length);
 const t=matches[0];return 'ice:'+f+':'+t.start+'-'+t.end+' (`'+t.name+'`)';
}
export const none='None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files. This entry concerns only the named in-process APIs/events.';
export const id=n=>'AGENT-'+String(n).padStart(3,'0');
export function add(spec,n,title,status,what,mechanism,interfaces,source,rust,deps=[],priority='P1',size=[60,120],extra={}){
 spec.entries.push({id:id(n),title,status,source,what,mechanism,interfaces,dependencies:deps.map(x=>typeof x==='number'?id(x):x),dependency_text:extra.external??'External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.',rust,port:extra.port??'Target ice-agent unless explicitly identified as an ice-harness or ice-tools helper. Preserve one authoritative loop. Use owned snapshots and explicit async error/cancellation contracts; native plugin consumers require the same host-facing hook/event data, not a second controller. Existing workspace dependencies tokio, tokio-util, futures and serde_json cover the primitives.',size_min:size[0],size_max:size[1],priority,priority_reason:extra.reason??'Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.'});
}
export function save(spec){
 const records=JSON.parse(fs.readFileSync(path.join(out,'evidence/tracked-files.json'),'utf8'));
 for(const r of records.filter(r=>r.area==='agent')){const actual=crypto.createHash('sha256').update(fs.readFileSync(path.join(root,r.path))).digest('hex');if(actual!==r.sha256)throw Error('Source changed since ledger: '+r.path);}
 fs.writeFileSync(path.join(out,'evidence/spec-agent.json'),JSON.stringify(spec,null,2)+'\n');console.log('Saved '+spec.entries.length+' source-checked agent entries.');
}
export function load(){return JSON.parse(fs.readFileSync(path.join(out,'evidence/spec-agent.json'),'utf8'));}
