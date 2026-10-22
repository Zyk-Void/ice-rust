import { auditHelperEvidence } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import {fileURLToPath} from "node:url";
const base=auditHelperEvidence;
const read=n=>JSON.parse(fs.readFileSync(path.join(base,n),"utf8"));
const hash=n=>crypto.createHash("sha256").update(fs.readFileSync(path.join(base,n))).digest("hex");
const original=read("tui-source-review.json"),close=read("tui-closeout-source-review.json"),term=read("tui-terminal-source-review.json");
const groups=[{file:"tui-source-review.json",key:"read_evidence",records:original.read_evidence},{file:"tui-source-review.json",key:"shared_read_evidence",records:original.shared_read_evidence},{file:"tui-closeout-source-review.json",key:"read_evidence",records:close.read_evidence},{file:"tui-terminal-source-review.json",key:"read_records",records:term.read_records}];
for(const f of ["tui-native-controller-supplement.json","tui-native-controller-test-supplement.json","tui-native-helper-supplement.json"])groups.push({file:f,key:"source_reads",records:read(f).source_reads});
groups.push({file:"misc-coordinator-docs-review.json",key:"read_evidence",records:read("misc-coordinator-docs-review.json").read_evidence});
const [mode="inspect"]=process.argv.slice(2);
if(mode==="inspect"){
 const records=groups.flatMap(g=>g.records.map((r,i)=>({file:g.file,index:i,...r})));
 const exclusions=records.filter(r=>Object.entries(r).some(([k,v])=>/block|exclud|redact/i.test(k)&&v&&(Array.isArray(v)?v.length:typeof v==="string"?v.length:true)));
 console.log(JSON.stringify({groups:groups.map(g=>({file:g.file,key:g.key,count:g.records.length,sample:g.records[0]})),exclusions,missingMetadata:records.filter(r=>!r.sha256||!(r.total_lines||r.line_count||r.totalLines)).map(r=>({file:r.file,path:r.path,keys:Object.keys(r)})),statusCorrection:close.status_test_metadata_correction,terminal_unread:term.unread_work,terminal_tools:term.tool_notes}));
}else if(mode==="claims"){
 const d=read("qa-first-pass-coverage.json");
 const isTui=r=>JSON.stringify(r).includes("TUI-");
 console.log(JSON.stringify({sha256:hash("qa-first-pass-coverage.json"),claims:d.claims.filter(isTui),accepted:d.accepted_claim_review.filter(isTui),uncovered:d.uncovered_claims,followup:d.follow_up_checks}));
}else throw Error("Unknown mode");
