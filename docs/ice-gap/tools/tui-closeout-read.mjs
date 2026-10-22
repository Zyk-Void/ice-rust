import { auditHelperEvidence } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import {fileURLToPath} from "node:url";
const base=auditHelperEvidence;
const read=n=>JSON.parse(fs.readFileSync(path.join(base,n),"utf8"));
const [mode="state",startText="0",countText="100"]=process.argv.slice(2);
const spec=read("spec-tui.json"),proposal=read("proposal-tui.json"),terminal=read("proposal-tui-terminal.json"),review=read("tui-source-review.json");
const candidates=read("candidates-tui.json");
const mapped=new Set([...proposal.surfaces,...terminal.surfaces].flatMap(r=>r.candidate_ids??(r.candidate_id?[r.candidate_id]:[])));
const excluded=new Set([...proposal.candidate_exclusions,...terminal.candidate_exclusions].map(r=>r.candidate_id));
const pending=candidates.filter(c=>!mapped.has(c.candidate_id)&&!excluded.has(c.candidate_id));
if(mode==="state"){
 const files=read("tracked-files.json").filter(r=>r.area==="tui");
 const reads=[...review.read_evidence,...review.shared_read_evidence];
 for(const n of ["tui-native-controller-supplement.json","tui-native-controller-test-supplement.json","tui-native-helper-supplement.json"]){const r=read(n);reads.push(...r.source_reads);}
 const byPath=new Map();for(const r of reads){if(!r.path)continue;const a=byPath.get(r.path)??[];a.push(r);byPath.set(r.path,a);}
 const missing=files.map(f=>{const rr=byPath.get("ice:"+f.path)??[];const lines=new Set();for(const r of rr)for(const[a,b]of r.visible_ranges??r.ranges??[])for(let n=a;n<=b;n++)lines.add(n);const gaps=[];for(let n=1;n<=f.lines;n++){if(lines.has(n))continue;let e=n;while(e+1<=f.lines&&!lines.has(e+1))e++;gaps.push([n,e]);n=e;}return{path:f.path,lines:f.lines,gaps};}).filter(f=>f.gaps.length);
 console.log(JSON.stringify({entries:spec.entries.length,proposal_entries:proposal.entries.length,ledger_rows:files.length,read_records:reads.length,missing,candidates:candidates.length,mapped:mapped.size,excluded:excluded.size,pending:pending.length,groups:Object.fromEntries([...new Set(pending.map(c=>c.path))].map(p=>[p,pending.filter(c=>c.path===p).length]))}));
}else if(mode==="pending"){
 const start=Number(startText),count=Number(countText);if(!Number.isInteger(start)||start<0||!Number.isInteger(count)||count<0||count>200)throw Error("Bad pagination");
 console.log(JSON.stringify({total:pending.length,start,rows:pending.slice(start,start+count)}));
}else if(mode==="source-records"){console.log(JSON.stringify(review.read_evidence));}
else throw Error("Unknown read-only mode");
