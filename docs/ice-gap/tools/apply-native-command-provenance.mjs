import { assertAuditOpen } from './lane-exports-rpc-ai-auth-completion-guard.mjs';
assertAuditOpen();
// One-time audit metadata correction; does not execute or modify application code.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {fileURLToPath} from 'node:url';
const base=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../evidence');
const file=path.join(base,'rust-defects.json'),before=fs.readFileSync(file);
const hash=crypto.createHash('sha256').update(before).digest('hex');
if(hash!=='b472f41620de1a72e5150a6d38fa3bb1ff8cb239bebed46c0767b949488afcce')throw Error('Historical correction already applied or input changed; do not replay.');
const rows=JSON.parse(before),patches=[
  {
    "id": "RUST-066",
    "provenance_status": "predates-local-rebrand",
    "provenance": "The reviewed command routing and handler logic are present in upstream bigfish1913/pi-rust at 0fb2d893206981527b14e86d68e3ddbdf2c8189a; every inspected line is blamed to upstream commit 539581cb90d2eb433dc4b73610d0948794995499. All significant texts match the corresponding rpi crate sources after crate-prefix normalization. No UI/subprocess execution was performed.",
    "provenance_evidence": "evidence/rust-defect-provenance.json",
    "provenance_review": {
      "upstream_commit": "0fb2d893206981527b14e86d68e3ddbdf2c8189a",
      "targets": [
        {
          "file": "crates/ice-cli/src/interactive_tui/commands.rs",
          "range": [
            24,
            41
          ],
          "upstream_file": "crates/rpi-cli/src/interactive_tui/commands.rs",
          "significant_lines_matched": 12,
          "significant_lines_reviewed": 12,
          "blame_commits": [
            {
              "sha": "539581cb90d2eb433dc4b73610d0948794995499",
              "in_upstream": true
            }
          ]
        },
        {
          "file": "crates/ice-cli/src/interactive_tui.rs",
          "range": [
            2996,
            2999
          ],
          "upstream_file": "crates/rpi-cli/src/interactive_tui.rs",
          "significant_lines_matched": 4,
          "significant_lines_reviewed": 4,
          "blame_commits": [
            {
              "sha": "539581cb90d2eb433dc4b73610d0948794995499",
              "in_upstream": true
            }
          ]
        }
      ],
      "method": "Independent coordinator source inspection plus local Git ancestry/source comparison."
    }
  },
  {
    "id": "RUST-067",
    "provenance_status": "predates-local-rebrand",
    "provenance": "The reviewed command routing and handler logic are present in upstream bigfish1913/pi-rust at 0fb2d893206981527b14e86d68e3ddbdf2c8189a; every inspected line is blamed to upstream commit 539581cb90d2eb433dc4b73610d0948794995499. All significant texts match the corresponding rpi crate sources after crate-prefix normalization. No UI/subprocess execution was performed.",
    "provenance_evidence": "evidence/rust-defect-provenance.json",
    "provenance_review": {
      "upstream_commit": "0fb2d893206981527b14e86d68e3ddbdf2c8189a",
      "targets": [
        {
          "file": "crates/ice-cli/src/interactive_tui.rs",
          "range": [
            3038,
            3040
          ],
          "upstream_file": "crates/rpi-cli/src/interactive_tui.rs",
          "significant_lines_matched": 3,
          "significant_lines_reviewed": 3,
          "blame_commits": [
            {
              "sha": "539581cb90d2eb433dc4b73610d0948794995499",
              "in_upstream": true
            }
          ]
        },
        {
          "file": "crates/ice-cli/src/interactive_tui/sessions.rs",
          "range": [
            48,
            75
          ],
          "upstream_file": "crates/rpi-cli/src/interactive_tui/sessions.rs",
          "significant_lines_matched": 23,
          "significant_lines_reviewed": 23,
          "blame_commits": [
            {
              "sha": "539581cb90d2eb433dc4b73610d0948794995499",
              "in_upstream": true
            }
          ]
        }
      ],
      "method": "Independent coordinator source inspection plus local Git ancestry/source comparison."
    }
  }
];
for(const p of patches){const row=rows.find(r=>r.id===p.id);if(!row)throw Error('Missing '+p.id);Object.assign(row,p);}
fs.writeFileSync(file,JSON.stringify(rows,null,2)+'\n');
fs.writeFileSync(path.join(base,'native-command-provenance-applied.json'),JSON.stringify({captured_at:new Date().toISOString(),input_sha256:hash,patches,source_only:true},null,2)+'\n');
console.log(JSON.stringify({defects:rows.length,updated:patches.map(p=>p.id),sha256:crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex')}));
