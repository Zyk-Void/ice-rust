// Validate authored audit JSON and source references without executing application code.
import fs from "node:fs";
import path from "node:path";
import {fileURLToPath} from "node:url";
const base=path.resolve(path.dirname(fileURLToPath(import.meta.url)),"..");
const repoRoot=path.resolve(base,"../..");
const area=process.argv[2];
if(!/^[a-z-]+$/.test(area??""))throw new Error("Invalid area");
const roots={ice:"/home/mewtwo/Zks/ice",rust:repoRoot};
const spec=JSON.parse(fs.readFileSync(path.join(base,"evidence","spec-"+area+".json"),"utf8"));
const proposal=JSON.parse(fs.readFileSync(path.join(base,"evidence","lane-final-areas-proposal-"+area+".json"),"utf8"));
const problems=[],refs=new Map(),counts=new Map();
const resolve=(repo,rel)=>{const p=path.resolve(roots[repo],rel);if(!p.startsWith(roots[repo]+path.sep))throw Error("Outside source root");return p;};
const lineCount=(repo,rel)=>{const key=repo+":"+rel;if(!counts.has(key))counts.set(key,fs.readFileSync(resolve(repo,rel),"utf8").split("\n").length);return counts.get(key);};
const scan=(text,label)=>{for(const m of text.matchAll(/\b(ice|rust):([A-Za-z0-9_./@+\-]+):(\d+)(?:-(\d+))?((?:,\d+(?:-\d+)?)*)/g)){const key=m[0];if(refs.has(key))continue;refs.set(key,label);try{const n=lineCount(m[1],m[2]);const ranges=[[+m[3],+(m[4]??m[3])],...m[5].split(",").filter(Boolean).map(x=>{const z=x.split("-").map(Number);return[z[0],z[1]??z[0]];})];for(const[a,b]of ranges)if(a<1||b<a||b>n)problems.push({kind:"citation_range",label,ref:key,actual_lines:n});}catch(e){problems.push({kind:"citation_file",label,ref:key,error:e.message});}}};
const walk=(x,label)=>{if(typeof x==="string")scan(x,label);else if(Array.isArray(x))x.forEach((v,i)=>walk(v,label+"["+i+"]"));else if(x&&typeof x==="object")for(const[k,v]of Object.entries(x))walk(v,label+"."+k);};
const ids=new Set();
for(const e of spec.entries){if(ids.has(e.id))problems.push({kind:"duplicate_id",id:e.id});ids.add(e.id);
for(const k of ["id","title","status","source","what","mechanism","interfaces","dependencies","rust","port","size_min","size_max","priority","priority_reason"])if(e[k]===undefined||e[k]===null||e[k]==="")problems.push({kind:"missing_field",id:e.id,field:k});
if(!["missing","partial","different-by-design"].includes(e.status))problems.push({kind:"status",id:e.id,value:e.status});
if(!/^P[0-3]$/.test(e.priority)||!Number.isInteger(e.size_min)||!Number.isInteger(e.size_max)||e.size_min<0||e.size_max<e.size_min)problems.push({kind:"priority_size",id:e.id});
if(!Array.isArray(e.dependencies))problems.push({kind:"dependencies",id:e.id});
const pe=proposal.entries.find(p=>p.id===e.id);if(!pe)problems.push({kind:"proposal_missing_entry",id:e.id});else for(const k of["title","status","priority","size_min","size_max","dependencies"])if(JSON.stringify(pe[k])!==JSON.stringify(e[k]))problems.push({kind:"proposal_entry_mismatch",id:e.id,field:k});
}
for(const e of proposal.entries)if(!ids.has(e.id))problems.push({kind:"proposal_extra_entry",id:e.id});
walk(spec,"spec");walk(proposal,"proposal");
for(const s of proposal.surfaces??[]){scan("ice:"+s.ice_ref,"surface:"+s.name);if(s.rust_ref)scan(s.rust_ref.startsWith("rust:")?s.rust_ref:"rust:"+s.rust_ref,"surface:"+s.name);}
for(const r of proposal.ledger??[])try{fs.statSync(resolve("ice",r.path));}catch(e){problems.push({kind:"ledger_file",path:r.path,error:e.message});}
for(const r of proposal.read_evidence??[]){const m=r.path?.match(/^(ice|rust):(.+)$/);if(!m){problems.push({kind:"read_path",path:r.path});continue;}try{const n=lineCount(m[1],m[2]);for(const[a,b]of r.ranges??[])if(a<1||b<a||b>n)problems.push({kind:"read_range",path:r.path,range:[a,b],actual_lines:n});for(const line of r.excluded_redacted_lines??[])if((r.ranges??[]).some(([a,b])=>line>=a&&line<=b))problems.push({kind:"redaction_coverage",path:r.path,line});}catch(e){problems.push({kind:"read_file",path:r.path,error:e.message});}}
console.log(JSON.stringify({area,entries:spec.entries.length,proposal_entries:proposal.entries.length,ledger_rows:proposal.ledger.length,surfaces:proposal.surfaces?.length,unique_source_references:refs.size,read_records:proposal.read_evidence?.length,problems},null,2));
if(problems.length)process.exitCode=1;
