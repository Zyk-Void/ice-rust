import { auditHelperBase } from './lane-exports-rpc-ai-auth-audit-paths.mjs';
import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
assertAuditOpen();
// Read-only source inventory. All generated audit files stay in docs/ice-gap.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { execFileSync } from 'node:child_process';

const tsRoot = '/home/mewtwo/Zks/ice';
const rustRoot = '/home/mewtwo/Zks/ice-rust';
const out = auditHelperBase;
const areas = ['ai','agent','tools','session','settings','extensions','rpc-remote','tui','subagents','safe-verify','observability','cognee','blackhole','providers-auth','cli-packaging','misc'];
const git = (root, args) => execFileSync('git', ['-C', root, ...args], {encoding:'utf8', maxBuffer:64*1024*1024});
const tracked = root => git(root, ['ls-files','-z']).split('\0').filter(Boolean).sort();
const csv = rows => rows.map(row => row.map(value => {
  const s = String(value ?? '');
  return /[",\r\n]/.test(s) ? '"' + s.replaceAll('"','""') + '"' : s;
}).join(',')).join('\n')+'\n';
function write(rel, content) {
  const file = path.resolve(out, rel);
  if (!file.startsWith(out + path.sep)) throw new Error('Outside audit directory');
  fs.mkdirSync(path.dirname(file), {recursive:true});
  fs.writeFileSync(file, content);
}
function areaFor(p) {
  if (p.includes('/examples/extensions/ice-blackhole/')) return 'blackhole';
  if (/\/ice-cognee(?:[./-])/.test(p) || p.endsWith('/ice-cognee.md')) return 'cognee';
  if (/\/ice-safe-verify\.ts$|\/ice-goals-host\.(ts|md)$/.test(p)) return 'safe-verify';
  if (p.includes('/subagents/') || /\/ice-(agent-packs|agent-view-bridge|mcp-bridge)\.ts$/.test(p) || /\/subagents-v2[^/]*\.md$/.test(p)) return 'subagents';
  if (p.includes('/observe/') || /\/(observability\.md|telemetry\.ts|timings\.ts)$/.test(p)) return 'observability';
  if (/^packages\/(client|protocol|server)\//.test(p) || p.includes('/modes/rpc/') || p.includes('/cli/experimental/') || p.includes('/coding-agent/src/client/') || p.endsWith('/rpc-entry.ts') || p.endsWith('/rpc.md')) return 'rpc-remote';
  if (/^packages\/tui\//.test(p) || p.includes('/modes/interactive/') || /\/(keybindings\.ts|footer-data-provider\.ts)$/.test(p)) return 'tui';
  if (p.startsWith('packages/ai/')) {
    if (/\/auth\/|\/providers\/|\/(env-api-keys|oauth|bun-oauth|bedrock-provider|model-catalog|models-store)\.ts$/.test(p)) return 'providers-auth';
    return 'ai';
  }
  if (p.includes('/core/extensions/') || /\/core\/(event-bus|ice-manifest)\.ts$/.test(p) || p.endsWith('/extensions.md')) return 'extensions';
  if (/\/core\/tools\/|\/harness\/tools\/|\/harness\/utils\/|\/core\/(bash-executor|exec|output-guard)\.ts$/.test(p)) return 'tools';
  if (p.startsWith('packages/storage/') || /\/harness\/(session|compaction)\//.test(p) || /\/core\/(agent-session[^/]*|session-[^/]+|messages|sdk|usage-totals|context-breakdown|cache-stats)\.ts$/.test(p) || p.includes('/core/compaction/') || /\/(compaction|session-format|sessions|sdk)\.md$/.test(p)) return 'session';
  if (p.startsWith('packages/agent/')) return 'agent';
  if (/\/core\/(auth[^/]*|model[^/]*|provider[^/]*|radius|remote-catalog-provider|runtime-credentials)\.ts$/.test(p) || /\/ice-provider[^/]*\.ts$/.test(p) || /\/(custom-provider|models|providers)\.md$/.test(p)) return 'providers-auth';
  if (/\/core\/(settings-manager|defaults|project-trust|trust-manager|resolve-config-value|resource-loader|skills|prompt-templates|system-prompt|source-info)\.ts$/.test(p) || /\/cli\/project-trust\.ts$/.test(p) || /\/(settings|skills|prompt-templates|security|environment-variables)\.md$/.test(p) || p.endsWith('/src/config.ts')) return 'settings';
  if (p.includes('/claude-sdk/') || p.includes('/extensions/llama/') || p.startsWith('packages/evals/') || p.endsWith('/llama-cpp.md')) return 'misc';
  if (p.includes('/src/cli/') || /\/src\/(cli|ice|main|package-manager-cli|migrations)\.ts$/.test(p) || p.includes('/src/bun/') || p.includes('/core/legacy-compat/') || p.endsWith('/core/package-manager.ts') || /\/(development|quickstart|usage|packages|shell-aliases|termux|windows)\.md$/.test(p)) return 'cli-packaging';
  return 'misc';
}
const exclude = p => /(^|\/)(test|tests|dist|node_modules)(\/|$)/.test(p) || /\.(test|spec)\.[cm]?[jt]sx?$/.test(p);
const required = p => /^packages\/[^/]+\/src\//.test(p) || /^packages\/coding-agent\/examples\/extensions\/ice-[^/]+\//.test(p) || p.startsWith('packages/coding-agent/docs/');
const supplemental = p => /^packages\/storage\/[^/]+\/src\//.test(p) || /^packages\/tui\/native\/[^/]+\/src\//.test(p);
const tsTracked = tracked(tsRoot);
const rustTracked = tracked(rustRoot);
const files = tsTracked.filter(p => (required(p) || supplemental(p)) && !exclude(p));
const records = files.map(p => {
  const buf = fs.readFileSync(path.join(tsRoot,p));
  const binary = buf.includes(0) || /\.(png|jpg|jpeg|gif|webp|ico|woff2?)$/i.test(p);
  const text = binary ? '' : buf.toString('utf8');
  const lines = binary ? 0 : text.split('\n').length - (text.endsWith('\n') ? 1 : 0);
  const generated = /(^|\/)(?:image-)?models\.generated\.ts$/.test(p) || (/\/providers\/data\//.test(p));
  const status = binary ? 'not-applicable' : 'pending';
  const notes = binary ? 'Documentation image asset; no executable behavior; bytes and hash recorded in evidence/tracked-files.json.' : generated ? 'Generated model data: retain one inventory row; generator and consumer behavior to be audited, not individual model values.' : supplemental(p) ? 'Supplemental scope: nested storage/native runtime source required to trace scoped behavior.' : '';
  return {path:p,lines,area:areaFor(p),entry_ids:'',status,notes,bytes:buf.length,sha256:crypto.createHash('sha256').update(buf).digest('hex'),generated,required:required(p)};
});
write('ledger.csv',csv([['path','lines','area','entry_ids','status','notes'],...records.map(r=>[r.path,r.lines,r.area,r.entry_ids,r.status,r.notes])]));
for (const area of areas) write(`evidence/ledger-${area}.csv`,csv([['path','lines','area','entry_ids','status','notes'],...records.filter(r=>r.area===area).map(r=>[r.path,r.lines,r.area,r.entry_ids,r.status,r.notes])]));
write('evidence/tracked-files.json',JSON.stringify(records,null,2)+'\n');
write('evidence/rust-tracked-files.json',JSON.stringify(rustTracked,null,2)+'\n');
write('evidence/ts-tests.txt',tsTracked.filter(p => /(^|\/)(test|tests)\//.test(p) || /\.(test|spec)\.[cm]?[jt]sx?$/.test(p)).join('\n')+'\n');
write('evidence/rust-files.txt',rustTracked.filter(p=>p.endsWith('.rs') || /Cargo\.(toml|lock)$/.test(p)).join('\n')+'\n');
const baseline = {
  captured_at:new Date().toISOString(),
  scope:'git-tracked current worktree contents; all requested files plus nested storage/native runtime dependencies; generated model data kept as rows, not expanded',
  ts:{root:tsRoot,head:git(tsRoot,['rev-parse','HEAD']).trim(),branch:git(tsRoot,['branch','--show-current']).trim(),status:git(tsRoot,['status','--short'])},
  rust:{root:rustRoot,head:git(rustRoot,['rev-parse','HEAD']).trim(),branch:git(rustRoot,['branch','--show-current']).trim(),status:git(rustRoot,['status','--short'])},
  ledger_rows:records.length,
  text_lines:records.reduce((n,r)=>n+r.lines,0),
  generated_rows:records.filter(r=>r.generated).length,
  area_counts:Object.fromEntries(areas.map(a=>[a,{files:records.filter(r=>r.area===a).length,lines:records.filter(r=>r.area===a).reduce((n,r)=>n+r.lines,0)}]))
};
write('evidence/baseline.json',JSON.stringify(baseline,null,2)+'\n');
console.log(JSON.stringify(baseline,null,2));
