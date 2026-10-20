import { auditHelperEvidence } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
if(["apply"].includes(process.argv[2]))assertAuditOpen();
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
const base=auditHelperEvidence;
const roots={ice:"/home/mewtwo/Zks/ice",rust:"/home/mewtwo/Zks/ice-rust"};
const specPath=path.join(base,"spec-extensions.json");
const baseline=JSON.parse(fs.readFileSync(path.join(base,"resume-source-identities-2026-10-19T222055Z.json"),"utf8"));
const spec=JSON.parse(fs.readFileSync(specPath,"utf8"));
const source=(repo,rel,a,b)=>{
 const full=path.resolve(roots[repo],rel);
 if(!full.startsWith(roots[repo]+"/"))throw Error("path outside source root");
 const body=fs.readFileSync(full,"utf8");
 const sha=crypto.createHash("sha256").update(body).digest("hex");
 const frozen=baseline.hashes[repo][rel];
 if(frozen&&frozen!==sha)throw Error("source drift "+repo+":"+rel);
 return body.split("\n").slice(+a-1,+b).join("\n");
};
const fence=String.fromCharCode(96).repeat(3);
const pattern=/\x60\x60\x60(?:ts|typescript|rust)\n([\s\S]*?)\n\x60\x60\x60\nSource: (ice|rust):([^:\n]+):(\d+)-(\d+)\./g;
const mismatches=[];
for(const entry of spec.entries){
 for(const m of entry.interfaces.matchAll(pattern)){
  const actual=source(m[2],m[3],m[4],m[5]);
  if(actual!==m[1])mismatches.push({id:entry.id,ref:m[2]+":"+m[3]+":"+m[4]+"-"+m[5]});
 }
}
if(process.argv[2]==="audit"){
 console.log(JSON.stringify({entries:spec.entries.length,title:spec.title,introduction:spec.introduction,appendix:spec.appendix,mismatches},null,2));
}else if(process.argv[2]==="apply"){
 const patches=JSON.parse(fs.readFileSync(path.join(base,"extensions-review-updates.json"),"utf8"));
 for(const patch of patches.entries){
  const found=spec.entries.find(e=>e.id===patch.id);
  if(found)Object.assign(found,patch);else spec.entries.push(patch);
 }
 if(patches.introduction!==undefined)spec.introduction=patches.introduction;
 if(patches.appendix!==undefined)spec.appendix=patches.appendix;
 for(const entry of spec.entries)if(patches.refresh_interfaces?.includes(entry.id)){
  entry.interfaces=entry.interfaces.replace(pattern,(_whole,_old,repo,rel,a,b)=>fence+(repo==="rust"?"rust":"ts")+"\n"+source(repo,rel,a,b)+"\n"+fence+"\nSource: "+repo+":"+rel+":"+a+"-"+b+".");
 }
 const serialized=JSON.stringify(spec,null,2)+"\n";
 JSON.parse(serialized);
 fs.writeFileSync(specPath,serialized);
 console.log(JSON.stringify({entries:spec.entries.length,sha256:crypto.createHash("sha256").update(serialized).digest("hex")}));
}else throw Error("expected audit or apply");
