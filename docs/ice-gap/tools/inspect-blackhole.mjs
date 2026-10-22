import { auditHelperBase } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
assertAuditOpen();
import fs from 'node:fs';
import path from 'node:path';
import {createRequire} from 'node:module';
const root='/home/mewtwo/Zks/ice';
const base=auditHelperBase;
const ts=createRequire(path.join(root,'package.json'))('typescript');
const printer=ts.createPrinter({removeComments:true,newLine:ts.NewLineKind.LineFeed});
const rows=JSON.parse(fs.readFileSync(path.join(base,'evidence/blackhole-copies.json'),'utf8'));
for(const r of rows){
 if(r.same_bytes===false&&r.source.endsWith('.ts')){
 const parsed=p=>ts.createSourceFile(p,fs.readFileSync(path.join(root,p),'utf8'),ts.ScriptTarget.Latest,true);
 r.same_printed_syntax=printer.printFile(parsed(r.source))===printer.printFile(parsed(r.copy));
 console.log(JSON.stringify({source:r.source,copy:r.copy,same_printed_syntax:r.same_printed_syntax}));
 }
}
fs.writeFileSync(path.join(base,'evidence/blackhole-copies.json'),JSON.stringify(rows,null,2)+'\n');
for(const s of JSON.parse(fs.readFileSync(path.join(base,'evidence/ts-symbols.json'),'utf8')).filter(s=>s.area==='blackhole'))console.log(s.path+':'+s.line+'-'+s.end_line+' '+s.name+' '+(s.exported?'exported':'private'));
console.log('Recall notice extra chars:', '\nUntrusted reference data recalled by Cognee:'.length);
