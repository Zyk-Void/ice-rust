import { auditHelperBase } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
assertAuditOpen();
// Refine mechanical inventories after inspecting declaration layouts. No source execution.
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
const tsRoot='/home/mewtwo/Zks/ice';
const root='/home/mewtwo/Zks/ice-rust';
const out=auditHelperBase;
const ts=createRequire(path.join(tsRoot,'package.json'))('typescript');
const records=JSON.parse(fs.readFileSync(path.join(out,'evidence/tracked-files.json'),'utf8'));
const candidates=JSON.parse(fs.readFileSync(path.join(out,'evidence/surface-candidates.json'),'utf8'));
const seen=new Set(candidates.map(c=>[c.category,c.name,c.path,c.line].join('|')));
const csv=rows=>rows.map(r=>r.map(v=>{const s=String(v??'');return /[",\r\n]/.test(s)?'"'+s.replaceAll('"','""')+'"':s;}).join(',')).join('\n')+'\n';
function save(p,s){const f=path.resolve(out,p);if(!f.startsWith(out+path.sep))throw Error('Output boundary');fs.mkdirSync(path.dirname(f),{recursive:true});fs.writeFileSync(f,s);}
const contracts=[];
for(const r of records){
 if(r.generated||!r.path.endsWith('.ts'))continue;
 const text=fs.readFileSync(path.join(tsRoot,r.path),'utf8');const sf=ts.createSourceFile(r.path,text,ts.ScriptTarget.Latest,true);
 const name=n=>n?.getText(sf)?.replace(/^['"]|['"]$/g,'')??'';
 function owner(n){for(let p=n.parent;p;p=p.parent)if((ts.isVariableDeclaration(p)||ts.isTypeAliasDeclaration(p)||ts.isInterfaceDeclaration(p)||ts.isFunctionDeclaration(p))&&p.name)return name(p.name);return '';}
 function add(category,nm,node,kind,detail=''){
  const line=sf.getLineAndCharacterOfPosition(node.getStart(sf)).line+1;const key=[category,nm,r.path,line].join('|');if(seen.has(key))return;seen.add(key);
  candidates.push({candidate_id:'C'+String(candidates.length+1).padStart(6,'0'),category,name:nm,path:r.path,line,end_line:sf.getLineAndCharacterOfPosition(node.getEnd()).line+1,area:r.area,kind,detail,rust_name_candidates:[]});
 }
 function walk(n){
  if(ts.isPropertyAssignment(n)){
   const o=owner(n);const k=name(n.name);
   if(['TUI_KEYBINDINGS','KEYBINDINGS'].includes(o)&&/^(tui|app)\./.test(k))add('keybindings',k,n,'registered-default-keybinding',n.initializer.getText(sf));
   if(o==='BUILTIN_SLASH_COMMANDS'&&k==='name'&&ts.isStringLiteral(n.initializer))add('slash-commands','/'+n.initializer.text,n,'builtin-slash-command');
   if(o==='KEYBINDING_NAME_MIGRATIONS')add('keybindings','legacy:'+k,n,'keybinding-name-migration',n.initializer.getText(sf));
  }
  if(ts.isTypeAliasDeclaration(n)&&/EventType|HookEvent|ObservationEvent|DecisionEvent|Telemetry/.test(name(n.name))){
   function lits(t){if(ts.isUnionTypeNode(t))return t.types.flatMap(lits);return ts.isLiteralTypeNode(t)&&ts.isStringLiteral(t.literal)?[t.literal.text]:[];}
   for(const v of lits(n.type))add(r.area==='observability'?'observability':'hooks',v,n,'event-type-union');
  }
  if(ts.isVariableDeclaration(n)&&n.initializer&&/EVENTS$/.test(name(n.name))){
   let init=n.initializer;while(ts.isAsExpression(init)||ts.isSatisfiesExpression(init))init=init.expression;
   if(ts.isArrayLiteralExpression(init))for(const e of init.elements)if(ts.isStringLiteral(e))add(r.area==='observability'?'observability':'hooks',e.text,e,'event-name-constant-array',name(n.name));
  }
  if(ts.isInterfaceDeclaration(n)||ts.isTypeAliasDeclaration(n))contracts.push({path:r.path,area:r.area,name:name(n.name),line:sf.getLineAndCharacterOfPosition(n.getStart(sf)).line+1,end_line:sf.getLineAndCharacterOfPosition(n.getEnd()).line+1,text:n.getText(sf)});
  ts.forEachChild(n,walk);
 }
 walk(sf);
}
save('evidence/surface-candidates.json',JSON.stringify(candidates,null,2)+'\n');
save('evidence/type-contracts.json',JSON.stringify(contracts,null,2)+'\n');
for(const area of [...new Set(records.map(r=>r.area))])save(`evidence/candidates-${area}.json`,JSON.stringify(candidates.filter(c=>c.area===area),null,2)+'\n');
const counts={};
for(const category of [...new Set(candidates.map(c=>c.category))]){
 const cs=candidates.filter(c=>c.category===category);counts[category]=cs.length;
 save(`surfaces/${category}.csv`,csv([['name','ice_path:line','rust_path:line_or_blank','status','entry_id'],...cs.map(c=>[c.name,c.path+':'+c.line,c.rust_name_candidates.join(';'),'pending',''])]));
}
const requirements=fs.readFileSync(path.join(out,'requirements.md'),'utf8');
if(!requirements.includes('## User override: no agents'))save('requirements.md',requirements+'\n## User override: no agents\nThe user subsequently instructed: "do not use any agents please". This overrides Method step 5. Continue directly without launching, delegating to, or resuming any agent. An earlier agent_plan call was dry-run only; no agent was launched. The preparation script was not used to delegate work.\n');
save('evidence/execution-policy.json',JSON.stringify({delegation:false,reason:'Explicit user override: do not use any agents please',source_writes_allowed:false,network_allowed:false,output_root:out},null,2)+'\n');
console.log(JSON.stringify({candidate_count:candidates.length,counts,type_contract_count:contracts.length,delegation:false},null,2));
