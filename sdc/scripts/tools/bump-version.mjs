/**
 * Dev-only: set the version everywhere it is written down, and print what changed.
 *
 * The version lives in five files and the release workflow reads it from the tag, so a bump that
 * misses one of them ships a build whose app, daemon and installer disagree about what they are.
 *
 *   node sdc/scripts/tools/bump-version.mjs 0.6.1
 */
import { readFileSync, writeFileSync } from 'node:fs';

const next = process.argv[2];

if (next === undefined || !/^\d+\.\d+\.\d+$/.test(next)) {
  console.error('usage: node sdc/scripts/tools/bump-version.mjs <major.minor.patch>');
  process.exit(1);
}

const targets = [
  { file: 'sdc/package.json', pattern: /"version": "[^"]+"/ },
  { file: 'sdc/app/package.json', pattern: /"version": "[^"]+"/ },
  { file: 'sdc/web/package.json', pattern: /"version": "[^"]+"/ },
  { file: 'sdc/cloud/package.json', pattern: /"version": "[^"]+"/ },
  { file: 'sdc/app/src-tauri/tauri.conf.json', pattern: /"version": "[^"]+"/ },
  { file: 'sdc/sdcd/Cargo.toml', pattern: /^version = "[^"]+"/m },
  { file: 'sdc/app/src-tauri/Cargo.toml', pattern: /^version = "[^"]+"/m },
];

for (const target of targets) {
  const before = readFileSync(target.file, 'utf8');
  const match = before.match(target.pattern);

  if (match === null) {
    console.error(`${target.file}: no version line matched - check the pattern`);
    process.exit(1);
  }

  const nextLine = match[0].startsWith('version') ? `version = "${next}"` : `"version": "${next}"`;

  writeFileSync(target.file, before.replace(target.pattern, nextLine));
  console.log(`${target.file}: ${match[0]} -> ${nextLine}`);
}