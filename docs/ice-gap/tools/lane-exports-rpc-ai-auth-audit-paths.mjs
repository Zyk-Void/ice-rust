// Pure path helpers for this audit tree; no source reads or writes.
import path from 'node:path';
import { fileURLToPath } from 'node:url';
export const auditHelperTools=path.dirname(fileURLToPath(import.meta.url));
export const auditHelperBase=path.dirname(auditHelperTools);
export const auditHelperEvidence=path.join(auditHelperBase,'evidence');
export const auditHelperNative=path.resolve(auditHelperBase,'../..');
