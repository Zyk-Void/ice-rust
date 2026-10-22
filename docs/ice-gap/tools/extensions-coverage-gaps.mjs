import { auditHelperEvidence } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
const base=auditHelperEvidence;
const roots={ice:"/home/mewtwo/Zks/ice",rust:"/home/mewtwo/Zks/ice-rust"};
const proposal=JSON.parse(fs.readFileSync(path.join(base,"proposal-extensions.json"),"utf8"));
const baseline=JSON.parse(fs.readFileSync(path.join(base,"resume-source-identities-2026-10-19T222055Z.json"),"utf8"));
const results=[];
for(const r of proposal.read_evidence){
 const split=r.path.indexOf(":");const repo=r.path.slice(0,split),rel=r.path.slice(split+1),full=path.resolve(roots[repo],rel);
 if(!full.startsWith(roots[repo]+"/"))throw Error("invalid root");
 const body=fs.readFileSync(full,"utf8");const sha=crypto.createHash("sha256").update(body).digest("hex");const n=body.split("\n").length;
 const missing=[];let next=1;for(const[a,b]of r.ranges){if(a>next)missing.push([next,a-1]);next=Math.max(next,b+1);}if(next<=n)missing.push([next,n]);
 results.push({path:r.path,total_lines:n,sha256:sha,recorded_sha_matches:sha===r.sha256,frozen_sha_matches:baseline.hashes[repo][rel]?sha===baseline.hashes[repo][rel]:null,read_ranges:r.ranges,unread_ranges:missing});
}
console.log(JSON.stringify({files:results.length,errors:results.filter(r=>!r.recorded_sha_matches||r.frozen_sha_matches===false),full:results.filter(r=>!r.unread_ranges.length).length,partial:results.filter(r=>r.unread_ranges.length),all:results},null,2));
