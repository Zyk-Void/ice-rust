// Bounded independent OBS QA: read-only Git identity checks, no application imports or tests.
import fs from 'node:fs';
import crypto from 'node:crypto';
import { spawnSync } from 'node:child_process';
const root='/home/mewtwo/Zks/ice';
const revision='3ab05193147f5abb8eb8ee6c0178f6dbce8918b0';
const files=['packages/coding-agent/test/observe-store.test.ts','packages/coding-agent/test/observe-cognee.test.ts','packages/coding-agent/test/observe-server.test.ts'];
const sha=b=>crypto.createHash('sha256').update(b).digest('hex');
const output=files.map(path=>{
 const current=fs.readFileSync(root+'/'+path);
 const historical=spawnSync('git',['show',revision+':'+path],{cwd:root,maxBuffer:1048576});
 const status=spawnSync('git',['status','--short','--',path],{cwd:root,encoding:'utf8'});
 if(status.status!==0)throw Error('status failed: '+path);
 return {path,current_sha256:sha(current),revision,revision_sha256:historical.status===0?sha(historical.stdout):null,revision_matches:historical.status===0?sha(current)===sha(historical.stdout):null,status:status.stdout.trim(),historical_read_error:historical.status===0?null:historical.stderr.toString().trim()};
});
console.log(JSON.stringify({source_only:true,tests_executed:false,files:output},null,2));
