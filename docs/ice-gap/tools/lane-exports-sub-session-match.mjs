import { auditHelperEvidence } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import fs from 'node:fs';
const E=auditHelperEvidence;const needles=process.argv.slice(2);
for(const f of fs.readdirSync(E).filter(f=>/^spec-[a-z-]+\.json$/.test(f))){const j=JSON.parse(fs.readFileSync(E+'/'+f,'utf8'));for(const e of j.entries??[]){if(needles.some(n=>JSON.stringify(e).includes(n)))console.log(JSON.stringify({file:f,id:e.id,title:e.title,source:e.source,mechanism:e.mechanism,interfaces:e.interfaces,rust:e.rust}));}}
