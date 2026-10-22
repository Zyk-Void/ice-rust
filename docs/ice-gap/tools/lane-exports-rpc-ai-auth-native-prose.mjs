import { auditHelperEvidence } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import fs from "node:fs";
import crypto from "node:crypto";
const dir=auditHelperEvidence+'/';
const areas=["ai","extensions","rpc-remote","providers-auth"];
const [mode,area,...args]=process.argv.slice(2);
const hash=(name)=>crypto.createHash("sha256").update(fs.readFileSync(dir+name)).digest("hex");
const read=(name)=>JSON.parse(fs.readFileSync(dir+name,"utf8"));
const pattern=/\bICE_[A-Z][A-Z0-9_]*\b|(?<![A-Za-z0-9_])\.ice(?![A-Za-z0-9_-])|(?<![A-Za-z0-9_:\/-])ice(?![A-Za-z0-9_:-])|ice:cancelled/g;
if(mode==="top"){
 for(const a of areas){
  const s=read("spec-"+a+".json"),crosslinks=[];
  for(const e of s.entries)for(const [field,text]of Object.entries(e))if(typeof text==="string"&&text.includes("RUST-045"))crosslinks.push({id:e.id,field,text});
  console.log(JSON.stringify({area:a,metadata:Object.fromEntries(Object.entries(s).filter(([k])=>k!=="entries")),crosslinks}));
 }
}
else if(mode==="inventory"){
 for(const a of areas){const name="spec-"+a+".json",s=read(name);console.log(JSON.stringify({area:a,file:dir+name,sha256:hash(name),top_keys:Object.keys(s),entries:s.entries.length,entry_keys:Object.keys(s.entries[0])}));}
 for(const name of ["ai-combined-read-manifest.json","auth-normalized-read-manifest.json","rust-prebuilt-rebrand-defect.json"]){const x=read(name);console.log(JSON.stringify({manifest:name,sha256:hash(name),kind:Array.isArray(x)?"array":"object",keys:Array.isArray(x)?null:Object.keys(x),length:Array.isArray(x)?x.length:null,metadata:Object.fromEntries(Object.entries(x).filter(([k,v])=>typeof v==="string"||typeof v==="number"||typeof v==="boolean"))}));}
}else{
 if(!areas.includes(area))throw new Error("area required");const name="spec-"+area+".json",s=read(name);
 if(mode==="scan"){
  for(const e of s.entries)for(const [field,v]of Object.entries(e))if(typeof v==="string"){const matches=[...v.matchAll(pattern)];if(matches.length)console.log(JSON.stringify({area,id:e.id,field,length:v.length,matches:matches.map(m=>({text:m[0],offset:m.index,context:v.slice(Math.max(0,m.index-130),Math.min(v.length,m.index+m[0].length+180))}))}));}
 }else if(mode==="entries"){for(const e of s.entries)if(args.includes(e.id))console.log(JSON.stringify(e));}
 else if(mode==="field"){const e=s.entries.find(e=>e.id===args[0]);console.log(JSON.stringify({area,id:e.id,field:args[1],text:e[args[1]],spec_sha256:hash(name)}));}
 else throw new Error("unknown mode");
}
