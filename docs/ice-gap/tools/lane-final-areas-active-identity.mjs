import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
if(["candidates","first-pass-findings"].includes(process.argv[2]))assertAuditOpen();
import fs from 'node:fs';import path from 'node:path';import{createHash}from'node:crypto';import{fileURLToPath}from'node:url';
const base=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..'),ev=path.join(base,'evidence'),args=process.argv.slice(2),read=n=>JSON.parse(fs.readFileSync(path.join(ev,n),'utf8')),hash=b=>createHash('sha256').update(b).digest('hex');
const areas=['misc','settings'],pattern=/\bICE_[A-Z0-9_]+|(?:~\/|\/|\b)\.ice(?:\/|\b)|\bice(?:\.exe)?\s+(?:--|-p|auth|install|package|dev|update)|\bZyk-Void\b/;
if(args[0]==='candidates'){
 const rows=[];
 for(const area of areas){const n='spec-'+area+'.json',b=fs.readFileSync(path.join(ev,n)),s=JSON.parse(b);for(const e of s.entries||s)for(const[field,text]of Object.entries(e))if(typeof text==='string'&&!['source','id','title'].includes(field)){const paras=text.split('\n\n').filter(t=>pattern.test(t));pattern.lastIndex=0;if(paras.length)rows.push({area,id:e.id,field,spec_sha256:hash(b),paragraphs:paras,full_text:text});}}
 const o={owner:'/root/final_area_handoffs',status:'mechanical-leads-not-reviewed',records:rows};fs.writeFileSync(path.join(ev,'lane-final-areas-active-identity-candidates.json'),JSON.stringify(o,null,2)+'\n');console.log(JSON.stringify({count:rows.length,index:rows.map((r,i)=>({index:i,area:r.area,id:r.id,field:r.field,paragraphs:r.paragraphs.length}))}));
}else if(args[0]==='show'){const rs=read('lane-final-areas-active-identity-candidates.json').records;console.log(JSON.stringify(rs.slice(+args[1],(+args[1])+(+args[2]||10)).map(({full_text,...r})=>r)))}
else if(args[0]==='diffs'){const h=read('active-checkout-diff-hunks-complete.json');for(const p of args.slice(1)){const r=h.rows.find(r=>r.old_path===p);console.log(JSON.stringify(r?{old_path:r.old_path,active_path:r.active_path,old_sha256:r.old.sha256,active_sha256:r.active.sha256,diff_sha256:r.diff_sha256,unified_diff:r.unified_diff}:{path:p,no_diff:true}));}}
else if(args[0]==='manifest-review'){
 for(const area of areas){const n=area+'.json',o=read(n);console.log(JSON.stringify({area,fields:Object.fromEntries(['rust_only','open_questions','unverified'].map(k=>[k,o[k]])),intro:fs.readFileSync(path.join(base,area+'.md'),'utf8').split('\n').slice(0,18)}))}
}
else if(args[0]==='native-paragraphs'){
 for(const area of areas){const o=read('spec-'+area+'.json');const rs=(o.entries||o).filter(e=>typeof e.rust==='string'&&/(?:\bice\b|ICE_|\.ice|Zyk-Void)/.test(e.rust));console.log(JSON.stringify({area,entries:rs.map(e=>({id:e.id,rust:e.rust}))}))}
}
else if(args[0]==='first-pass-findings'){
 const x=read('qa-first-pass-coverage.json');const records=x.claims.map((r,i)=>({index:i,id:r.id,finding:r.finding,evidence:r.evidence})).filter(r=>/RUST-045|Zyk-Void|rebrand|prebuilt|self.update|ICE_[A-Z_]+|\.ice(?:[\/\s'"`]|$)|\bice(?:\.exe)?\s+(?:--|-p|update|install|auth|package)/i.test(r.finding||''));
 fs.writeFileSync(path.join(ev,'lane-final-areas-first-pass-findings-candidates.json'),JSON.stringify({owner:'/root/final_area_handoffs',status:'review-pending',records},null,2)+'\n');console.log(JSON.stringify(records));
}
else throw new Error('unsupported');
