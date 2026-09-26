// Temporary compatibility fix for cospec's affected embedded OpenSpec bundle.
// Its vendor build combines bin/openspec.js (unconditional runCli()) with
// dist/cli/index.js (runCli() when argv[1] equals import.meta.url). Bundling
// collapses both module paths, so the extracted entry executes twice.
// SHA256: 0d2dfc42f009c9827c4a035cccf0ad8ceec6d0670a9fa78094e1939cb25830c1.
// This known content hash identifies the affected bundle. Give only its
// redundant self-main guard an inert argv[1]; command arguments stay intact
// and the unconditional entry still runs every original cospec gate.
// Remove this file and its task-scoped BUN_OPTIONS after a pinned upstream
// release fixes vendoring and the actual standalone contract tests pass.
// No external Bun or OpenSpec installation is used: cospec owns this runtime.
const path = require('node:path');

const entry = process.argv[1];
if (
  process.env.BUN_BE_BUN === '1' &&
  entry &&
  path.basename(entry) === 'openspec.js' &&
  path.basename(path.dirname(entry)) === 'bin' &&
  path.basename(path.dirname(path.dirname(entry))) === 'vendor' &&
  path.basename(path.dirname(path.dirname(path.dirname(entry)))) ===
    'openspec-1.13.1-0d2dfc42f009c982'
) {
  process.argv[1] = path.join(path.dirname(entry), 'cospec-openspec-entry.js');
}
