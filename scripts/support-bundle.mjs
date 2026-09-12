import { writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import path from 'node:path';
import { collectSupportDiagnostics } from '../electron/support-diagnostics.mjs';

const output = process.argv[2];
if (!output) throw new Error('Usage: node scripts/support-bundle.mjs OUTPUT.json');
const root = process.env.AYLA_DATA_DIR || path.join(process.env.XDG_CONFIG_HOME || path.join(homedir(), '.config'), 'ayla');
writeFileSync(path.resolve(output), JSON.stringify(collectSupportDiagnostics(root), null, 2) + '\n', { mode: 0o600, flag: 'wx' });
console.log('Redacted support diagnostics exported. Review the JSON before sharing it.');
