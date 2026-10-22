// Read-only inspection of audit artifacts and their cited source lines.
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
const base=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const [mode,area,...ids]=process.argv.slice(2);
if(!/^[a-z-]+$/.test(area??''))throw Error('Area required');
const read=n=>JSON.parse(fs.readFileSync(path.join(base,'evidence',n),'utf8'));
if(mode==='entry'){
const s=read('spec-'+area+'.json');
for(const e of s.entries)if(!ids.length||ids.includes(e.id))console.log(JSON.stringify(e,null,2));
}else if(mode==='surfaces'){
const p=read('proposal-'+area+'.json');
const ice='/home/mewtwo/Zks/ice';
const cache=new Map();
for(const s of p.surfaces??[]){
const m=s.ice_ref.match(/^(.*):(\d+)$/);if(!m)throw Error(s.ice_ref);
if(!cache.has(m[1]))cache.set(m[1],fs.readFileSync(path.join(ice,m[1]),'utf8').split('\n'));
console.log(JSON.stringify({name:s.name,ref:s.ice_ref,line:cache.get(m[1])[+m[2]-1],entry:s.entry_id}));
}
}else if(mode==='summary'){
const s=read('spec-'+area+'.json');console.log(JSON.stringify(s.entries.map(({id,title,status,dependencies})=>({id,title,status,dependencies})),null,2));
}else throw Error('Unknown mode');
