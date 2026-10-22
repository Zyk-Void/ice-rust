import { auditHelperEvidence } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import fs from "node:fs";
import crypto from "node:crypto";
const dir=auditHelperEvidence+'/';
const load=(name)=>JSON.parse(fs.readFileSync(dir+name,"utf8"));
const digest=(name)=>crypto.createHash("sha256").update(fs.readFileSync(dir+name)).digest("hex");
if(process.argv[2]==="rename"){
 const name="active-checkout-source-map.json",m=load(name),arrays=Object.entries(m).filter(([k,v])=>Array.isArray(v));
 console.log(JSON.stringify({artifact:"evidence/"+name,sha256:digest(name),root_fields:Object.keys(m),arrays:arrays.map(([name,rows])=>({name,total:rows.length,matches:rows.filter(r=>JSON.stringify(r).includes("crates/ice-cli/src/bin/ice"))}))},null,2));
}else if(process.argv[2]==="headers"){
 const areas=["agent","ai","blackhole","cli-packaging","cognee","extensions","misc","observability","providers-auth","rpc-remote","safe-verify","session","settings","subagents","tools","tui"];
 for(const area of areas){
  const name="spec-"+area+".json",s=load(name);
  console.log(JSON.stringify({area,path:dir+name,sha256:digest(name),actual_entry_count:s.entries.length,status_counts:s.entries.reduce((a,e)=>(a[e.status]=(a[e.status]??0)+1,a),{}),metadata:Object.fromEntries(Object.entries(s).filter(([k])=>k!=="entries"))}));
 }
}else if(process.argv[2]==="proposal-state"){
 const p=load("lane-exports-rpc-ai-auth-native-prose-replacements.json");
 console.log(JSON.stringify({proposal_sha256:digest("lane-exports-rpc-ai-auth-native-prose-replacements.json"),rows:p.replacements.map(r=>{
  const name="spec-"+r.area+".json",s=load(name),e=s.entries.find(x=>x.id===r.entry_id);
  return {area:r.area,id:r.entry_id,field:r.field,current_spec_sha256:digest(name),matches_old:e[r.field]===r.old_text,matches_new:e[r.field]===r.new_text};
 })},null,2));
}else throw new Error("mode required");
