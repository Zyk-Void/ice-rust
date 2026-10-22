import { auditHelperEvidence } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
const allowed = new Set(["spec-tui-terminal.json","proposal-tui-terminal.json","tui-terminal-source-review.json","cli-source-review.json","qa-session-test-closure.json","spec-tui.json","proposal-tui.json","tui-source-review.json","spec-extensions.json","tui-palette-catalog.json"]);
const [file,mode="meta",key="",startArg="0",countArg="20"] = process.argv.slice(2);
if(!allowed.has(file)) throw new Error("Not an allowed audit evidence file");
const root=auditHelperEvidence;
const text=fs.readFileSync(path.join(root,file),"utf8");
const sha256=crypto.createHash("sha256").update(text).digest("hex");
const data=JSON.parse(text);
if(mode==="meta") {
 console.log(JSON.stringify({file,sha256,bytes:Buffer.byteLength(text),keys:Object.keys(data),arrays:Object.fromEntries(Object.entries(data).filter(([,v])=>Array.isArray(v)).map(([k,v])=>[k,v.length]))}));
} else if(mode==="value") {
 let v=data;
 for(const k of key.split(".").filter(Boolean)) {if(v===undefined||v===null)throw new Error("Missing key"); v=v[k];}
 const start=Number(startArg),count=Number(countArg);
 if(!Number.isInteger(start)||!Number.isInteger(count)||start<0||count<0||count>200)throw new Error("Bad pagination");
 const value=Array.isArray(v)?v.slice(start,start+count):v;
 console.log(JSON.stringify({file,sha256,key,total:Array.isArray(v)?v.length:undefined,start,value}));
} else if(mode==="entry-summaries") {
 console.log(JSON.stringify({file,sha256,entries:data.entries.map(({id,title,status,what,mechanism,dependencies})=>({id,title,status,what,opening:mechanism.split("\n\n")[0],dependencies}))}));
} else throw new Error("Unknown read-only mode");
