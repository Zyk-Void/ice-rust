// Independent artifact-only remap review. Does not import or execute the remapper.
import fs from'node:fs';import path from'node:path';import{createHash}from'node:crypto';import{fileURLToPath}from'node:url';
const B=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..'),E=path.join(B,'evidence');
const hash=x=>createHash('sha256').update(x).digest('hex'),read=n=>JSON.parse(fs.readFileSync(path.join(E,n),'utf8'));
const P=read('relocation-rust-remap-preview.json'),M=read('active-checkout-source-map.json'),D=read('active-checkout-diff-hunks-complete.json'),C=read('read-coverage.json');
const errors=[],notes=[],inputs=[],check=(yes,msg,details)=>{if(!yes)errors.push({message:msg,details});return yes;},same=(a,b)=>JSON.stringify(a)===JSON.stringify(b);
function input(rel){const b=fs.readFileSync(path.join(B,rel));const r={path:rel,sha256:hash(b)};inputs.push(r);return r;}
const tool=input('tools/remap-rust-citations.mjs'),preview=input('evidence/relocation-rust-remap-preview.json');
const reviewedToolSha=process.argv[2];if(!reviewedToolSha||tool.sha256!==reviewedToolSha)throw Error('Pass the independently read tool SHA to bind this review');
check(P.remapper_sha256===tool.sha256,'Preview was not produced by the independently reviewed tool');
check(P.mode==='preview'&&P.source_only&&!P.application_or_test_execution,'Wrong preview mode or source-only scope');
const toolText=fs.readFileSync(path.join(B,tool.path),'utf8'),bindingMatch=toolText.match(/for\(const k of(\[[^\]]+\])\)if\(JSON\.stringify\(preview\[k\]\)!==JSON\.stringify\(report\[k\]\)\)/);
const boundKeys=bindingMatch?JSON.parse(bindingMatch[1].replaceAll("'",'"')):[];
const requiredBoundKeys=['old_root','active_root','old_head','active_head','source_map_sha256','hunk_map_sha256','remapper_sha256','review_inputs','inputs','files','source_identities','already_active_source_identities','summary','citations','canonical_read_remap'];
check(requiredBoundKeys.every(k=>boundKeys.includes(k)),'Apply is not fully bound to the reviewed preview',boundKeys);
check(toolText.includes('historical_coordinate_indexes'),'Historical coordinate index annotation absent');
check(P.source_map_sha256===input('evidence/active-checkout-source-map.json').sha256,'Source map differs from preview');
check(P.hunk_map_sha256===input('evidence/active-checkout-diff-hunks-complete.json').sha256,'Hunk map differs from preview');
const coverageHash=input('evidence/read-coverage.json').sha256;
const reviewObjects=new Map();
for(const i of P.review_inputs){const a=input(i.path);check(i.sha256===a.sha256,'Review input differs from preview',i.path);reviewObjects.set(i.path.split('/').at(-1),read(i.path.split('/').at(-1)));}
const oldRows=new Map(M.rows.map(r=>[r.old_path,r])),activeRows=new Map(M.rows.filter(r=>r.active_path).map(r=>[r.active_path,r])),diffRows=new Map(D.rows.map(r=>[r.old_path,r]));
const approvals=new Map(),ckey=c=>[c.old_start,c.old_count,c.active_start,c.active_count].join(':');
function addApproval(p,c,oldHash,activeHash,confirmed,artifact){const m=oldRows.get(p);check(confirmed,'Unconfirmed review segment',{p,key:ckey(c),artifact});check(m?.old?.sha256===oldHash&&m?.active?.sha256===activeHash,'Review source identities differ',{p,artifact});const a=approvals.get(p)||new Map();a.set(ckey(c),artifact);approvals.set(p,a);}
for(const[n,r]of reviewObjects){
 if(n.startsWith('coordinator-'))for(const f of r.files)for(const h of f.hunks)addApproval(f.old_path,h,f.old_sha256,f.active_sha256,h.confirmed,n);
 else if(n.includes('rpc-ai-auth-native-hunk'))for(const f of r.files)for(const h of f.hunks)for(const c of h.changed_ranges)addApproval(f.old_path,c,f.old_sha256,f.active_sha256,['confirmed','confirmed-with-documentation-caveat'].includes(h.review_status)&&!(h.redactions||[]).length,n);
 else if(n.includes('sub-session-native-hunk'))for(const f of r.rows)for(const c of f.changes)addApproval(f.old_path,c,f.old.sha256,f.active.sha256,c.reviewed,n);
 else if(n==='lane-final-areas-active-noncrates-review.json')for(const c of r.reviews)addApproval(c.old_path,c,c.old_sha256,c.active_sha256,!!c.confirmation,n);
}
const cache=new Map();
function source(root,rel){if(path.isAbsolute(rel)||rel.split('/').includes('..')||/^(?:\.ice|\.icers|\.git|\.aws|\.ssh)(?:\/|$)/.test(rel))throw Error('Unsafe source metadata path');
 const absolute=path.join(root,rel);if(cache.has(absolute))return cache.get(absolute);
 const b=fs.readFileSync(absolute),ls=[];let cursor=0,pos;
 while((pos=b.indexOf(10,cursor))!==-1){ls.push({bytes:b.subarray(cursor,pos),terminated:true});cursor=pos+1;}
 if(cursor<b.length)ls.push({bytes:b.subarray(cursor),terminated:false});
 const out={sha256:hash(b),lines:ls.length,split_lines:ls.length+(cursor===b.length?1:0),ls,valid_utf8:Buffer.from(b.toString('utf8')).equals(b)};
 check(out.valid_utf8,'Non-roundtripping UTF8 makes text comparison insufficient',rel);cache.set(absolute,out);return out;
}
const fileModels=new Map(),zeroSegments=[];
function model(rel){if(fileModels.has(rel))return fileModels.get(rel);
 const m=oldRows.get(rel);if(!m?.old?.present||!m?.active?.present)throw Error('No two-sided map '+rel);
 const old=source(P.old_root,rel),active=source(P.active_root,m.active_path),cs=['identical','renamed_identical'].includes(m.status)?[]:(diffRows.get(rel)?.changes||[]);
 check(old.sha256===m.old.sha256&&active.sha256===m.active.sha256,'Fixed source identity mismatch',rel);
 const lm=new Map(),cm=new Map(),spans=[];let oi=1,ni=1;
 function unchanged(count){if(count<0)throw Error('Negative unchanged segment '+rel);
   for(let k=0;k<count;k++){const ol=old.ls[oi+k-1],nl=active.ls[ni+k-1];check(!!ol&&!!nl&&ol.bytes.equals(nl.bytes)&&ol.terminated===nl.terminated,'Unequal bytes outside captured changes',{rel,old_line:oi+k,active_line:ni+k});lm.set(oi+k,ni+k);}if(count)spans.push({old:[oi,oi+count-1],active:[ni,ni+count-1]});oi+=count;ni+=count;}
 for(const c of cs){const ob=c.old_start+(c.old_count===0?1:0),nb=c.active_start+(c.active_count===0?1:0);
   check(ob-oi===nb-ni,'Old/active unchanged block length differs',{rel,c,old_cursor:oi,active_cursor:ni});
   unchanged(ob-oi);
   check(approvals.get(rel)?.has(ckey(c)),'Changed segment has no matching source-reviewed approval',{rel,key:ckey(c)});
   for(let x=0;x<c.old_count;x++)cm.set(ob+x,c);
   oi=ob+c.old_count;ni=nb+c.active_count;
   if(c.old_count===0||c.active_count===0)zeroSegments.push({path:rel,...c,review:approvals.get(rel)?.get(ckey(c))});
 }
 check(old.lines-oi===active.lines-ni,'Final unchanged block length differs',{rel,old_cursor:oi,active_cursor:ni});
 unchanged(Math.max(0,old.lines-oi+1));
 if(old.split_lines>old.lines&&active.split_lines>active.lines)lm.set(old.split_lines,active.split_lines);
 const result={rel,active_path:m.active_path,old,active,cs,lm,cm,spans,status:m.status};fileModels.set(rel,result);return result;
}
function mappedEndpoint(d,n,end=false){const c=d.cm.get(n);if(c){if(!c.active_count)throw Error('Deleted endpoint requires review '+d.rel+':'+n);return end?c.active_start+c.active_count-1:c.active_start;}const out=d.lm.get(n);if(out===undefined)throw Error('No independent endpoint '+d.rel+':'+n);return out;}
function independentRange(d,a,b){const nr=[mappedEndpoint(d,a),mappedEndpoint(d,b,true)];const overlap=d.cs.filter(c=>c.old_count?Math.max(a,c.old_start)<=Math.min(b,c.old_start+c.old_count-1):a<=c.old_start&&c.old_start<b);
 return{range:nr,method:overlap.length||d.rel!==d.active_path?'reviewed':nr[0]!==a||nr[1]!==b?'shifted':'unchanged',overlap:overlap.map(ckey)};}
const citationChecks=[],ambiguousSurfacePoints=[],protectedMappings=[],activeAliasRecords=[];
const protectedKey=/(?:^|\/)(?:report_excerpt|report_refs|reference_normalization|read_sources|source_review|read_evidence|reads|read_coverage|hashes|inputs|provenance|provenance_targets|provenance_review|provenance_evidence|frozen_checkout_finding|baseline_identity_limitations|source_label_repairs)(?:\/|$)/;
for(const c of P.citations){
 if(protectedKey.test(c.location))protectedMappings.push({document:c.document,location:c.location,old:c.old_citation,next:c.new_citation});
 if(c.original_coordinate_space==='active'||c.old_citation.startsWith('rust-active:')){
   const a=source(P.active_root,c.active_path),m=activeRows.get(c.active_path);
   check(a.sha256===m?.active?.sha256&&c.verified_active_sha256===a.sha256,'Already-active alias hash mismatch',c.active_path);
   check(same(c.old_range,c.new_range)&&c.method==='unchanged','Already-active alias shifted',c);
   check(c.new_range[0]>=1&&c.new_range[1]<=a.split_lines,'Already-active alias out of bounds',c);
   activeAliasRecords.push({document:c.document,location:c.location,old:c.old_citation,next:c.new_citation,sha256:a.sha256});continue;
 }
 const d=model(c.old_path),e=independentRange(d,...c.old_range);
 check(same(e.range,c.new_range)&&e.method===c.method,'Independent citation range/method mismatch',{document:c.document,location:c.location,old:c.old_citation,expected:e,actual:{range:c.new_range,method:c.method}});
 check(same(e.overlap,(c.overlapping_changes||[]).map(ckey)),'Overlap review set differs',{old:c.old_citation});
 const expected='rust:'+d.active_path+':'+e.range[0]+(/[-–]\d+$/.test(c.old_citation)||e.range[0]!==e.range[1]?'-'+e.range[1]:'');
 const isSurface=/\/surfaces\/\d+\/rust_ref$/.test(c.location);
 check(c.new_citation===(isSurface?'rust:'+d.active_path+':'+e.range[0]:expected),'Rendered citation differs from independently calculated range',c.old_citation);
 if(isSurface&&e.range[0]!==e.range[1])ambiguousSurfacePoints.push({document:c.document,location:c.location,old:c.old_citation,next:c.new_citation,reviewed_range:e.range});
}
check(protectedMappings.length===0,'Historical/provenance fields present in remap preview',protectedMappings);
function expand(rs){const s=new Set();for(const[a,b]of rs)for(let i=a;i<=b;i++)s.add(i);return s;}
function ranges(s){const xs=[...s].sort((a,b)=>a-b),o=[];for(const x of xs){if(o.length&&x===o.at(-1)[1]+1)o.at(-1)[1]=x;else o.push([x,x]);}return o;}
function excluded(d,ls){const out=new Set();for(const l of ls){const[a,b]=independentRange(d,l,l).range;for(let i=a;i<=b;i++)out.add(i);}return out;}
const coverageChecks=[];let unchangedCredit=0,reviewCredit=0,allOldMissing=0,allNewMissing=0,unreadUnchangedPromoted=0;
for(const r of P.canonical_read_remap){
 const old=C.files[r.old_key],rel=r.old_key.slice(5),d=model(rel);
 check(!!old,'Preview coverage key missing from original',r.old_key);if(!old)continue;
 check(old.sha256===d.old.sha256&&r.old_sha256===d.old.sha256&&r.active_sha256===d.active.sha256,'Coverage source identity mismatch',rel);
 check(same(old.ranges,r.old_ranges)&&same(old.excluded_redacted_lines||[],r.old_redacted)&&same(old.excluded_unverified_lines||[],r.old_unverified),'Preview old read/exclusion ranges differ',rel);
 const oldCredit=expand(old.ranges),oldEx=new Set([...(old.excluded_redacted_lines||[]),...(old.excluded_unverified_lines||[])]),u=new Set(),fresh=new Set();
 for(const l of oldCredit)if(l<=d.old.lines&&!oldEx.has(l)&&d.lm.has(l))u.add(d.lm.get(l));
 for(const c of d.cs)for(let i=0;i<c.active_count;i++){const n=c.active_start+i;if(n<=d.active.lines)fresh.add(n);}
 const newRed=excluded(d,old.excluded_redacted_lines||[]),newUnv=excluded(d,old.excluded_unverified_lines||[]),newEx=new Set([...newRed,...newUnv]),credit=new Set([...u,...fresh]);for(const n of newEx)credit.delete(n);
 const missingOld=new Set(),missingNew=new Set();let promoted=0;
 for(let l=1;l<=d.old.lines;l++)if(!oldCredit.has(l)&&!oldEx.has(l)){missingOld.add(l);if(d.lm.has(l)&&credit.has(d.lm.get(l)))promoted++;}
 for(let l=1;l<=d.active.lines;l++)if(!credit.has(l)&&!newEx.has(l))missingNew.add(l);
 check(same(ranges(credit),r.new_ranges),'Independent new read credit differs',rel);
 check(same([...newRed].sort((a,b)=>a-b),r.new_redacted)&&same([...newUnv].sort((a,b)=>a-b),r.new_unverified),'Redaction/unverified exclusions differ',rel);
 check(same(ranges(missingNew),r.ordinary_missing_ranges),'Ordinary missing ranges differ',rel);
 check(promoted===0,'Unread unchanged line was promoted to read',rel);
 check(![...newEx].some(n=>credit.has(n)),'Excluded line has read credit',rel);
 const uc=[...u].filter(n=>!newEx.has(n)).length,rc=[...fresh].filter(n=>!newEx.has(n)&&!u.has(n)).length;
 unchangedCredit+=uc;reviewCredit+=rc;allOldMissing+=missingOld.size;allNewMissing+=missingNew.size;unreadUnchangedPromoted+=promoted;
 coverageChecks.push({old_key:r.old_key,new_key:r.new_key,old_sha256:d.old.sha256,active_sha256:d.active.sha256,unchanged_credit_lines:uc,reviewed_change_credit_lines:rc,credit_lines:credit.size,old_ordinary_missing_lines:missingOld.size,new_ordinary_missing_lines:missingNew.size,old_redacted:r.old_redacted,new_redacted:r.new_redacted,old_unverified:r.old_unverified,new_unverified:r.new_unverified,ordinary_missing_ranges:r.ordinary_missing_ranges,source_semantic_review_claimed:false,independent_range_check_passed:promoted===0});
}
const uniqueNativePaths=new Set([...(P.source_identities||[]).map(r=>r.active_path),...(P.already_active_source_identities||[]).map(r=>r.active_path)]);
check(P.summary.source_files===uniqueNativePaths.size,'Summary source_files omits validated active paths',{reported:P.summary.source_files,unique:uniqueNativePaths.size});
const expectedRustKeys=Object.keys(C.files).filter(k=>k.startsWith('rust:')).sort();
check(same(expectedRustKeys,P.canonical_read_remap.map(r=>r.old_key).sort()),'Native canonical files omitted or duplicated');
const firstPass=read('qa-first-pass-coverage.json'),aliases={},aliasSamples=[];
for(const c of firstPass.claims)for(const text of[c.finding||'',...(c.evidence||[])])for(const m of String(text).matchAll(/\b([A-Za-z][A-Za-z0-9_-]*):([A-Za-z0-9_./@+\-]+):(\d+)(?:[-–](\d+))?/g)){aliases[m[1]]=(aliases[m[1]]||0)+1;if(m[1].startsWith('rust')&&m[1]!=='rust')aliasSamples.push({id:c.id,citation:m[0]});}
const staleDocuments=[];
for(const i of P.inputs){const f=input(i.path);if(f.sha256!==i.sha256)staleDocuments.push({path:i.path,preview_sha256:i.sha256,current_sha256:f.sha256});}
if(staleDocuments.length)notes.push({kind:'fresh-preview-required',message:'The coordinator has changed comparison documents since this preview; regenerate after applying pending findings. Range/coverage checks still verify the captured snapshot.',files:staleDocuments});
if(aliasSamples.length&&!activeAliasRecords.length)notes.push({kind:'planned-fix',message:'The reviewed base tool omitted rust-active aliases. Coordinator is adding hash/bounds validation and normalization without shifting, using original_coordinate_space=active.',aliases:aliasSamples});
const nonCrateModel=model('ICE_PORTING.md'),insertionRegression={old_659:independentRange(nonCrateModel,659,659),old_660:independentRange(nonCrateModel,660,660),span_659_660:independentRange(nonCrateModel,659,660)};
check(same(insertionRegression.old_659.range,[659,659])&&same(insertionRegression.old_660.range,[688,688])&&same(insertionRegression.span_659_660.range,[659,688]),'Known zero-old insertion regression differs',insertionRegression);
const missingChanges=coverageChecks.filter(r=>r.old_ordinary_missing_lines!==r.new_ordinary_missing_lines).map(r=>({path:r.old_key,old_missing:r.old_ordinary_missing_lines,new_missing:r.new_ordinary_missing_lines,remaining:r.ordinary_missing_ranges}));
const exclusionChanges=coverageChecks.filter(r=>r.old_redacted.length||r.old_unverified.length).map(r=>({path:r.old_key,old_redacted:r.old_redacted,new_redacted:r.new_redacted,old_unverified:r.old_unverified,new_unverified:r.new_unverified}));
const canonicalFieldShapes=[...new Set(Object.entries(C.files).filter(([k])=>k.startsWith('rust:')).map(([,v])=>Object.keys(v).sort().join(',')))];
const staticReview={point_and_range_math:'The independent span model reproduces all preview endpoints; zero-old changes add offset only after their old anchor and are reviewed only when a range crosses that anchor.',read_credit:'Unchanged transfer excludes all replaced old lines and compares source payload bytes and terminators. Separately reviewed active changed lines are added, then all mapped redaction and unverified lines are removed.',historical_fields:'First-pass report_excerpt/claim/report_refs are explicitly preserved; no mapped preview citation appears inside protected provenance/read fields. Historical TS baseline hashes and original native read_sources remain separate from active file identities.',source_only:'The reviewed tool only reads source/Git identity and changes audit files in the active audit copy; preview writes only its audit preview artifact.',freshness_guard:'The final apply guard binds document inputs, tool/source/hunk/review hashes, roots/heads, source identities, planned before/after hashes, summary, citations and canonical remap to the independently reviewed preview. Mode and capture timestamp are intentionally excluded.',active_alias_fix:'The final tool validates rust-active references against active presence, file hash and range bounds and normalizes the label without shifting coordinates. They do not add canonical read credit.',historical_indexes:'Retained rust_non_crates and legacy_only_records are explicitly identified as historical coordinate indexes; current read credit is in files and historical read_sources are preserved.'};
const result={schema_version:1,owner:'/root/final_area_handoffs',captured_at:new Date().toISOString(),status:errors.length?'needs-correction':notes.some(n=>n.kind==='planned-fix'||n.kind==='fresh-preview-required')?'snapshot-passed-refresh-required':'passed',source_only:true,application_or_test_execution:false,reviewed_tool:tool,preview,method:'Independent unchanged-block reconstruction from saved minimal old/new segments; compare actual source line bytes as metadata, independently map every citation and rebuild every native read-credit set. No application source semantic reread or new read credit is claimed by this verifier.',summary:{citation_occurrences:P.citations.length,old_coordinate_citations:P.citations.length-activeAliasRecords.length,already_active_citations:activeAliasRecords.length,unique_old_citations:new Set(P.citations.map(c=>c.old_citation)).size,native_coverage_files:coverageChecks.length,unique_validated_native_paths:uniqueNativePaths.size,source_models:fileModels.size,unchanged_credit_lines:unchangedCredit,reviewed_change_credit_lines:reviewCredit,old_ordinary_missing_lines:allOldMissing,new_ordinary_missing_lines:allNewMissing,unread_unchanged_lines_promoted:unreadUnchangedPromoted,zero_count_segments:zeroSegments.length,protected_field_mappings:protectedMappings.length,ambiguous_surface_points:ambiguousSurfacePoints.length,historical_missing_baseline_hashes:(C.baseline_identity_limitations||[]).filter(r=>!r.baseline_worktree_sha256).length,errors:errors.length},static_review:staticReview,apply_preview_bound_keys:boundKeys,missing_changes:missingChanges,exclusion_preservation:exclusionChanges,canonical_native_record_field_shapes:canonicalFieldShapes,insertion_regression:insertionRegression,zero_count_segments:zeroSegments,active_alias_records:activeAliasRecords,first_pass_citation_namespaces:aliases,first_pass_native_aliases:aliasSamples,coverage_checks:coverageChecks,ambiguous_surface_points:ambiguousSurfacePoints,protected_field_mappings:protectedMappings,notes,errors,inputs};
const name='lane-final-areas-remap-review.json',s=JSON.stringify(result,null,2)+'\n';fs.writeFileSync(path.join(E,name),s);console.log(JSON.stringify({path:'evidence/'+name,sha256:hash(s),status:result.status,summary:result.summary,first_pass_citation_namespaces:aliases,first_pass_native_aliases:aliasSamples,insertion_regression:insertionRegression,missing_changes:missingChanges,exclusion_preservation:exclusionChanges,canonical_native_record_field_shapes:canonicalFieldShapes,ambiguous_surface_points:ambiguousSurfacePoints,notes,errors},null,2));
