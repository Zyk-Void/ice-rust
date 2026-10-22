import { auditHelperEvidence } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import {spawnSync} from "node:child_process";
const base=auditHelperEvidence;
const roots={ice:"/home/mewtwo/Zks/ice",rust:"/home/mewtwo/Zks/ice-rust"};
const sha=b=>crypto.createHash("sha256").update(b).digest("hex");
const ev=JSON.parse(fs.readFileSync(path.join(base,"extensions-source-review.json"),"utf8"));
const spec=JSON.parse(fs.readFileSync(path.join(base,"spec-extensions.json"),"utf8"));
const mode=process.argv[2];
if(mode==="manifest"){
 const m=JSON.parse(fs.readFileSync(path.join(base,"agent.json"),"utf8"));
 console.log(JSON.stringify({keys:Object.keys(m),complete:m.complete,entries:m.entries.slice(0,2),ledger:m.ledger.slice(0,1),surfaces:m.surfaces.slice(0,3),candidate_exclusions:m.candidate_exclusions?.slice(0,3),claims:m.claims?.slice(0,2),dependency_requests:m.dependency_requests?.slice(0,2),rust_only:m.rust_only?.slice(0,2),open_questions:m.open_questions?.slice(0,2),read_evidence:m.read_evidence?.slice(0,2)},null,2));
}else if(mode==="tests"){
 const seen=new Set();const result=[];
 for(const e of ev.read_evidence){
  if(!e.path.startsWith("ice:")||!e.path.includes("/test/")||seen.has(e.path))continue;
  seen.add(e.path);const rel=e.path.slice(4);
  const full=path.resolve(roots.ice,rel);if(!full.startsWith(roots.ice+"/packages/coding-agent/test/"))throw Error("test outside scope");
  const current=fs.readFileSync(full);
  const baseline=spawnSync("git",["-C",roots.ice,"show","3ab05193147f5abb8eb8ee6c0178f6dbce8918b0:"+rel],{encoding:null,maxBuffer:2000000});
  const status=spawnSync("git",["-C",roots.ice,"status","--short","--untracked-files=all","--",rel],{encoding:"utf8",maxBuffer:100000});
  result.push({path:e.path,current_sha256:sha(current),recorded_read_sha256:e.sha256,read_hash_matches:sha(current)===e.sha256,git_status:status.stdout.trim(),tracked_at_frozen_head:baseline.status===0,frozen_head_sha256:baseline.status===0?sha(baseline.stdout):null,matches_frozen_head:baseline.status===0?sha(baseline.stdout)===sha(current):null,limitations:baseline.status!==0||sha(baseline.stdout)!==sha(current)?"Current test evidence only; historical dirty-content identity not invented.":null});
 }
 console.log(JSON.stringify(result,null,2));
}else if(mode==="refs"){
 const re=/(ice|rust):([^\s\x60;,()\[\]]+?):(\d+)-(\d+)/g;
 const missing=[];const tests=[];const unreads=[];
 for(const e of spec.entries){
  for(const [field,text] of Object.entries(e)){
   if(typeof text!=="string")continue;
   for(const m of text.matchAll(re)){
    const ref=m[1]+":"+m[2],a=+m[3],b=+m[4],f=path.resolve(roots[m[1]],m[2]);
    if(!f.startsWith(roots[m[1]]+"/"))throw Error("invalid ref");
    if(!fs.existsSync(f)){missing.push({id:e.id,field,ref:m[0]});continue;}
    if(m[2].includes("/test/"))tests.push({id:e.id,field,ref:m[0]});
    const ranges=ev.read_evidence.filter(r=>r.path===ref).flatMap(r=>r.ranges).sort((x,y)=>x[0]-y[0]);
    let next=a;for(const [start,end]of ranges){if(start<=next&&end>=next)next=end+1;}
    if(next<=b)unreads.push({id:e.id,field,ref:m[0],first_unread:next});
   }
  }
 }
 console.log(JSON.stringify({missing,tests,unreads},null,2));
}else throw Error("mode");
