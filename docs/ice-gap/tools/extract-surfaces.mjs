import { auditHelperBase } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
assertAuditOpen();
// Mechanical candidate extraction only. Candidates are not behavioral evidence.
// No dependencies are installed, no source is written, and no network is used.
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
const root='/home/mewtwo/Zks/ice';
const rustRoot='/home/mewtwo/Zks/ice-rust';
const out=auditHelperBase;
const req=createRequire(path.join(root,'package.json'));
let ts;
try { ts=req('typescript'); } catch (error) { throw new Error('Existing TypeScript parser unavailable; no installation attempted: '+error.message); }
const files=JSON.parse(fs.readFileSync(path.join(out,'evidence/tracked-files.json'),'utf8'));
const rustFiles=JSON.parse(fs.readFileSync(path.join(out,'evidence/rust-tracked-files.json'),'utf8')).filter(p=>p.endsWith('.rs'));
const categories=['cli','slash-commands','keybindings','settings','environment','tools','rpc','extension-api','session-types','paths','providers-auth','tui','hooks','observability'];
const candidates=[];
const symbols=[];
const seen=new Set();
const rust=[];
const csv=rows=>rows.map(row=>row.map(v=>{const s=String(v??'');return /[",\n\r]/.test(s)?'"'+s.replaceAll('"','""')+'"':s;}).join(',')).join('\n')+'\n';
function put(rel,data){const f=path.resolve(out,rel);if(!f.startsWith(out+path.sep))throw Error('Outside output root');fs.mkdirSync(path.dirname(f),{recursive:true});fs.writeFileSync(f,data);}
function add(category,name,record,node,source,kind,detail='') {
  if (!name || !categories.includes(category)) return;
  const line=source.getLineAndCharacterOfPosition(node.getStart(source)).line+1;
  const end_line=source.getLineAndCharacterOfPosition(node.getEnd()).line+1;
  const key=[category,name,record.path,line].join('|');
  if(seen.has(key))return;seen.add(key);
  candidates.push({candidate_id:'C'+String(candidates.length+1).padStart(6,'0'),category,name,path:record.path,line,end_line,area:record.area,kind,detail});
}
function enclosing(node,predicate){for(let n=node.parent;n;n=n.parent)if(predicate(n))return n;}
function textName(n,s){return n?.getText(s)?.replace(/^['"]|['"]$/g,'')??'';}
for(const record of files){
  if(record.generated || !/\.[cm]?[jt]sx?$/.test(record.path))continue;
  const text=fs.readFileSync(path.join(root,record.path),'utf8');
  const source=ts.createSourceFile(record.path,text,ts.ScriptTarget.Latest,true,/\.jsx?$/.test(record.path)?ts.ScriptKind.JS:ts.ScriptKind.TS);
  const constants=new Map();
  function collect(n){if(ts.isVariableDeclaration(n)&&ts.isIdentifier(n.name)&&n.initializer)constants.set(n.name.text,n.initializer);ts.forEachChild(n,collect);}collect(source);
  function literal(n,depth=0){
    if(!n||depth>8)return undefined;
    if(ts.isStringLiteralLike(n)||ts.isNumericLiteral(n))return n.text;
    if(ts.isIdentifier(n)&&constants.has(n.text))return literal(constants.get(n.text),depth+1);
    if(ts.isAsExpression(n)||ts.isParenthesizedExpression(n)||ts.isSatisfiesExpression?.(n))return literal(n.expression,depth+1);
    return undefined;
  }
  function nearestDecl(n){const d=enclosing(n,x=>ts.isInterfaceDeclaration(x)||ts.isTypeAliasDeclaration(x)||ts.isVariableDeclaration(x)||ts.isClassDeclaration(x)||ts.isFunctionDeclaration(x));return d?.name?textName(d.name,source):'';}
  function visit(n){
    if((ts.isFunctionDeclaration(n)||ts.isClassDeclaration(n)||ts.isInterfaceDeclaration(n)||ts.isTypeAliasDeclaration(n)||ts.isEnumDeclaration(n))&&n.name){
      const exported=!!n.modifiers?.some(m=>m.kind===ts.SyntaxKind.ExportKeyword);
      const line=source.getLineAndCharacterOfPosition(n.getStart(source)).line+1;
      const end_line=source.getLineAndCharacterOfPosition(n.getEnd()).line+1;
      symbols.push({path:record.path,area:record.area,name:textName(n.name,source),kind:ts.SyntaxKind[n.kind],line,end_line,exported});
      if(record.area==='tui'&&(ts.isClassDeclaration(n)||exported&&ts.isFunctionDeclaration(n)))add('tui',textName(n.name,source),record,n,source,'component-or-rendering-export');
    }
    if(ts.isCallExpression(n)){
      const expression=n.expression.getText(source);
      const method=ts.isPropertyAccessExpression(n.expression)?n.expression.name.text:expression;
      const value=literal(n.arguments[0]);
      if(method==='registerFlag')add('cli',value!==undefined?'--'+value:'<dynamic registerFlag: '+n.arguments[0]?.getText(source)+'>',record,n,source,'registerFlag',n.getText(source));
      if(method==='registerCommand')add('slash-commands',value!==undefined?'/'+value:'<dynamic registerCommand>',record,n,source,'registerCommand',n.getText(source));
      if(method==='registerShortcut')add('keybindings',value??'<dynamic registerShortcut>',record,n,source,'registerShortcut',n.getText(source));
      if(method==='registerTool'){
        const arg=n.arguments[0];let name;
        if(arg&&ts.isObjectLiteralExpression(arg))for(const p of arg.properties)if(ts.isPropertyAssignment(p)&&textName(p.name,source)==='name')name=literal(p.initializer);
        add('tools',name??'<dynamic registerTool: '+arg?.getText(source).slice(0,100)+'>',record,n,source,'registerTool',arg?.getText(source)??'');
      }
      if(method==='on'&&value!==undefined&&(expression.startsWith('pi.')||expression.startsWith('ice.')||/extension|subagent|cognee|safe-verify|observe|blackhole/.test(record.path)))add('hooks',value,record,n,source,'hook-registration');
      if(['appendEntry','appendCustomEntry'].includes(method))add('session-types',value??'<dynamic customType: '+n.arguments[0]?.getText(source)+'>',record,n,source,method,n.getText(source));
      if(/^(getEnv|readEnv|envVar|readProviderEnv|getProviderEnv|getLegacyEnv)$/.test(method)&&value!==undefined)add('environment',value,record,n,source,'environment-helper',expression);
      if(/^(emit|emitEvent|recordEvent|track|trackEvent|capture|record|observe)$/.test(method)&&value!==undefined&&(/observe|telemetry|subagents/.test(record.path)))add('observability',value,record,n,source,'event-emission',expression);
      if(/^(readFileSync|readFile|writeFileSync|writeFile|mkdirSync|mkdir|appendFileSync|appendFile|openSync|readdirSync|readdir|statSync|stat|unlinkSync|unlink|rm|renameSync|rename)$/.test(method))add('paths',n.arguments[0]?.getText(source)??'<unknown>',record,n,source,'filesystem-target-expression',expression);
    }
    if(ts.isPropertyAccessExpression(n)&&/^(process|Bun)\.env$/.test(n.expression.getText(source)))add('environment',n.name.text,record,n,source,'direct-env-read');
    if(ts.isElementAccessExpression(n)&&/^(process|Bun)\.env$/.test(n.expression.getText(source)))add('environment',literal(n.argumentExpression)??'<dynamic env: '+n.argumentExpression.getText(source)+'>',record,n,source,'indexed-env-read');
    if(ts.isBindingElement(n)&&ts.isVariableDeclaration(n.parent?.parent)&&/^(process|Bun)\.env$/.test(n.parent.parent.initializer?.getText(source)??''))add('environment',textName(n.propertyName??n.name,source),record,n,source,'destructured-env-read');
    if(ts.isStringLiteralLike(n)){
      const value=n.text;
      if(/^--[a-z][a-z0-9-]*(?:=.+)?$/i.test(value)&&(/\/cli\/|\/ice\.ts$|\/main\.ts$|\/subagents\/|\/ice-safe-verify\.ts$/.test(record.path)))add('cli',value,record,n,source,'flag-literal');
      if(value.includes('.ice/')||value==='\.ice'||value==='~/.ice'||value==='.ice')add('paths',value,record,n,source,'ice-path-literal');
      const p=n.parent;
      if(ts.isPropertyAssignment(p)){
        const key=textName(p.name,source);
        if(key==='customType')add('session-types',value,record,p,source,'customType');
        if(key==='name'&&(/\/tools\/|\/subagents\/api\//.test(record.path)))add('tools',value,record,p,source,'tool-name-object');
        if(key==='api'&&(/\/ai\/|provider|model/.test(record.path)))add('providers-auth','api:'+value,record,p,source,'wire-api-value');
        if(key==='id'&&(/\/ai\/src\/providers\/|\/auth\/oauth\//.test(record.path)))add('providers-auth','provider:'+value,record,p,source,'provider-id');
        if(key==='type'&&record.path.includes('/modes/rpc/'))add('rpc',value,record,p,source,'rpc-type-object');
      }
    }
    if(ts.isPropertySignature(n)||ts.isMethodSignature(n)){
      const owner=enclosing(n,x=>ts.isInterfaceDeclaration(x)||ts.isTypeAliasDeclaration(x));
      const ownerName=owner?.name?.text??'';
      const name=textName(n.name,source);
      if(/^(ExtensionAPI|ExtensionContext|ExtensionCommandContext|ExtensionUIContext|ExtensionRuntime|ReadonlySessionManager|ToolDefinition|ExtensionSettings)/.test(ownerName))add('extension-api',ownerName+'.'+name,record,n,source,'interface-member',n.getText(source));
      if(/Settings|Config|Appearance/.test(ownerName)&&(/settings|appearance|ice-cognee|ice-provider/.test(record.path)))add('settings',ownerName+'.'+name,record,n,source,'settings-type-member',n.getText(source));
      if(name==='type'&&n.type){
        const getTypes=t=>ts.isUnionTypeNode(t)?t.types.flatMap(getTypes):ts.isLiteralTypeNode(t)&&ts.isStringLiteral(t.literal)?[t.literal.text]:[];
        for(const v of getTypes(n.type)){
          if(record.path.includes('/modes/rpc/')||record.path==='packages/protocol/src/schemas.ts')add('rpc',v,record,n,source,ownerName||'rpc-type');
          if(/session-manager|harness\/session\/types|core\/messages/.test(record.path))add('session-types',v,record,n,source,ownerName||'session-type');
          if(record.path.includes('/core/extensions/types.ts'))add('hooks',v,record,n,source,ownerName||'extension-event');
          if(record.path.includes('/observe/'))add('observability',v,record,n,source,ownerName||'observe-event');
        }
      }
    }
    if(ts.isPropertyAssignment(n)){
      const decl=nearestDecl(n);
      if(/DEFAULT.*KEYBINDINGS|DEFAULT.*BINDINGS/.test(decl))add('keybindings',textName(n.name,source),record,n,source,'default-keybinding-action',n.initializer.getText(source));
      if(/DEFAULT.*SETTINGS|DEFAULT.*CONFIG|DEFAULT_ICE/.test(decl))add('settings',decl+'.'+textName(n.name,source),record,n,source,'default-setting',n.initializer.getText(source));
    }
    ts.forEachChild(n,visit);
  }
  visit(source);
}
for(const p of rustFiles){
  let text;try{text=fs.readFileSync(path.join(rustRoot,p),'utf8');}catch{continue;}
  const lines=text.split('\n');
  for(let i=0;i<lines.length;i++){
    const line=lines[i];
    for(const m of line.matchAll(/(?:std::)?env::var(?:_os)?\(\s*"([^"]+)"/g))rust.push({category:'environment',name:m[1],path:p,line:i+1,kind:'env-read'});
    for(const m of line.matchAll(/--([a-z][a-z0-9-]*)/g))rust.push({category:'cli',name:'--'+m[1],path:p,line:i+1,kind:'literal-flag-candidate'});
    for(const m of line.matchAll(/\bpub\s+(?:async\s+)?(?:fn|struct|enum|trait)\s+(\w+)/g))rust.push({category:'symbol',name:m[1],path:p,line:i+1,kind:'public-rust-symbol'});
    if(/#\[arg\(/.test(line))rust.push({category:'cli',name:line.trim(),path:p,line:i+1,kind:'clap-attribute'});
  }
}
for(const c of candidates){c.rust_name_candidates=rust.filter(r=>r.category===c.category&&r.name===c.name).map(r=>r.path+':'+r.line);}
put('evidence/surface-candidates.json',JSON.stringify(candidates,null,2)+'\n');
put('evidence/ts-symbols.json',JSON.stringify(symbols,null,2)+'\n');
put('evidence/rust-surface-candidates.json',JSON.stringify(rust,null,2)+'\n');
for(const area of [...new Set(files.map(f=>f.area))])put(`evidence/candidates-${area}.json`,JSON.stringify(candidates.filter(c=>c.area===area),null,2)+'\n');
for(const category of categories){
  const rows=candidates.filter(c=>c.category===category).map(c=>[c.name,c.path+':'+c.line,c.rust_name_candidates.join(';'),'pending','']);
  put(`surfaces/${category}.csv`,csv([['name','ice_path:line','rust_path:line_or_blank','status','entry_id'],...rows]));
}
const counts=Object.fromEntries(categories.map(c=>[c,candidates.filter(x=>x.category===c).length]));
put('evidence/mechanical-extraction.json',JSON.stringify({typescript_version:ts.version,candidate_count:candidates.length,category_counts:counts,symbol_count:symbols.length,rust_candidate_count:rust.length,warning:'Candidate extraction only. Exact identifiers suggest lookup locations, not behavioral equivalence. Final surface rows must be confirmed by reading source and tests; dynamic expressions must be resolved or explicitly explained.'},null,2)+'\n');
console.log(JSON.stringify({typescript_version:ts.version,candidate_count:candidates.length,counts,symbols:symbols.length,rust_candidates:rust.length},null,2));
