import { auditHelperEvidence } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import {fileURLToPath} from "node:url";
const base=auditHelperEvidence;
const read=n=>JSON.parse(fs.readFileSync(path.join(base,n),"utf8"));
const hash=n=>crypto.createHash("sha256").update(fs.readFileSync(path.join(base,n))).digest("hex");
const [mode="inspect",query=""]=process.argv.slice(2);
const names=["spec-tui.json","proposal-tui.json","proposal-tui-terminal.json","tui-source-review.json","tui-closeout-source-review.json","tui-terminal-source-review.json","tui-native-controller-supplement.json","tui-native-controller-test-supplement.json","tui-native-helper-supplement.json","misc-coordinator-docs-review.json","qa-first-pass-coverage.json"];
if(mode==="inspect"){
 const p=read("proposal-tui.json"),t=read("proposal-tui-terminal.json"),s=read("spec-tui.json"),tracked=read("tracked-files.json").filter(x=>x.area==="tui");
 const led=new Map([...p.ledger,...t.ledger].map(x=>[x.path,x]));
 console.log(JSON.stringify({artifacts:names.map(n=>{const d=read(n);return{name:n,sha256:hash(n),keys:Object.keys(d),arrays:Object.fromEntries(Object.entries(d).filter(([,v])=>Array.isArray(v)).map(([k,v])=>[k,v.length]))}}),missingLedger:tracked.filter(x=>!led.has(x.path)),supplemental:t.supplemental_ledger.slice(0,3),mainShared:read("tui-source-review.json").shared_read_evidence.slice(-3),entryCount:s.entries.length}));
}else if(mode==="records"){
 const out=[];const walk=(v,where,file)=>{if(!v||typeof v!=="object")return;if(!Array.isArray(v)&&typeof v.path==="string"&&v.path.includes(query)&&((v.ranges||v.visible_ranges||v.range)))out.push({file,where,record:v});for(const[k,x]of Object.entries(v))if(x&&typeof x==="object")walk(x,where+"."+k,file);};
 for(const n of names)walk(read(n),"$",n);console.log(JSON.stringify(out));
}else if(mode==="claims"){const d=read("qa-first-pass-coverage.json");console.log(JSON.stringify(d));}
else throw Error("Unknown read-only mode");
