#!/usr/bin/env node
// Does `protocol/types.ts` still agree with `sdcp.schema.json`? (0.7.10)
//
//   node protocol/check.mjs
//
// `types.ts` is the file every line of the app imports and `sdcp.schema.json` is the file the daemon and the
// spec are written against, and until now nothing compared them - so a method could be added to one and not
// the other, and the drift showed up as a `Property 'x' does not exist` in a *later* change, or worse, as a
// call the daemon answers `unknown method` to.
//
// This is a **drift check, not a generator**. The README used to say a generator would make the two unable to
// drift, and that is true - but `types.ts` carries the prose that explains each method (why `fs.read` caps at
// a megabyte, why `session.fork` replays its turns) and a generator would write that prose away. A check gets
// the property that matters - they cannot disagree silently - and keeps the documentation.
//
// What it compares:
//
//   1. every name in the schema's `methods` list is a key of `SdcpMethods` in `types.ts`, and nothing in
//      `types.ts` is missing from the schema;
//   2. every event type the schema lists is a member of the `EventType` union in `types.ts`;
//   3. every method the *daemon* dispatches (`"x.y" =>` arms) is in both.
//
// Exit code 0 means they agree; 1 lists what differs.
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '..');

const schema = JSON.parse(readFileSync(join(here, 'sdcp.schema.json'), 'utf8'));
const types = readFileSync(join(here, 'types.ts'), 'utf8');
const methods_rs = readFileSync(join(root, 'sdcd', 'src', 'sdcp', 'methods.rs'), 'utf8');

const schemaMethods = new Set(schema.$defs.methodNames.items?.enum ?? schema.$defs.methodNames.enum);
const schemaMethodSchemas = new Set(Object.keys(schema.$defs.methods.properties ?? {}));
const schemaEvents = new Set(schema.$defs.eventTypes.items?.enum ?? schema.$defs.eventTypes.enum);

/** Everything that did not line up. Declared before the readers, because one of them records problems too. */
const problems = [];

/**
 * `types.ts` with its comments taken out, for the readers below.
 *
 * The union blocks are *documented*, and a comment inside them carries apostrophes, semicolons and the word
 * `|` - which is how the first version of this check read a sentence out of a doc comment as a method name.
 * Comments are not code: strip them, then read the code.
 */
const stripped = types.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/[^\n]*/g, '');

/** The `SdcpMethod` union, member by member - the names a caller may send. It ends at its first `;`. */
const unionStart = stripped.indexOf('export type SdcpMethod =');
const unionBody = stripped.slice(unionStart, stripped.indexOf(';', unionStart));
const typeMethods = new Set([...unionBody.matchAll(/'([^']+)'/g)].map((match) => match[1]));

/** `SdcpMethodMap`'s keys - the ones that carry `params` and `result`. */
const mapBody = stripped.slice(
  stripped.indexOf('export interface SdcpMethodMap'),
  stripped.indexOf('export interface SdcpTransport'),
);
const mapKeys = new Set([...mapBody.matchAll(/^\s{2}'([^']+)':/gm)].map((match) => match[1]));

/**
 * Event names, from the interfaces `SdcpEvent` is made of: each declares `type: 'Name'`.
 *
 * Read through the union rather than by scanning every `*Event` interface, because a name in `SdcpEvent` is the
 * thing a daemon can push and an interface that is not in the union is not.
 */
const eventUnionStart = stripped.indexOf('export type SdcpEvent =');
const eventUnionBody = stripped.slice(eventUnionStart, stripped.indexOf(';', eventUnionStart));
const eventTypes = new Set();

for (const match of eventUnionBody.matchAll(/\| (\w+Event)/g)) {
  const declaration = new RegExp(`export interface ${match[1]} \\{[^}]*?type: '([^']+)';`, 's').exec(stripped);

  if (declaration === null) {
    problems.push(`SdcpEvent names \`${match[1]}\` and there is no such interface with a \`type\``);
  } else {
    eventTypes.add(declaration[1]);
  }
}

/** The arms the daemon actually answers - including `"a" | "b" =>` pairs, which are one arm and two names. */
const dispatched = new Set(
  [...methods_rs.matchAll(/^\s+((?:"[a-z][a-zA-Z.]*"(?:\s*\|\s*)?)+)\s*=>/gm)].flatMap((arm) =>
    [...arm[1].matchAll(/"([a-z][a-zA-Z.]*)"/g)].map((name) => name[1]),
  ),
);

for (const method of schemaMethods) {
  if (!typeMethods.has(method)) {
    problems.push(`the schema declares \`${method}\` and types.ts does not`);
  }

  if (!mapKeys.has(method)) {
    problems.push(`\`${method}\` has no \`params\`/\`result\` shape in SdcpMethodMap`);
  }
}

for (const method of typeMethods) {
  if (!schemaMethods.has(method)) {
    problems.push(`types.ts has \`${method}\` and the schema does not`);
  }

  if (!dispatched.has(method)) {
    problems.push(`types.ts declares \`${method}\` and the daemon has no arm for it`);
  }
}

for (const name of schemaEvents) {
  if (!eventTypes.has(name)) {
    problems.push(`the schema lists event \`${name}\` and types.ts has no interface for it`);
  }
}

for (const name of schemaMethodSchemas) {
  if (!schemaMethods.has(name)) {
    problems.push(`\`${name}\` has a shape in the schema without being named in methodNames`);
  }
}

/* A name with no shape is a method the schema mentions and does not describe: a caller reading the schema
   cannot know what to send. Reported here because this check is the only thing that reads the file. */
for (const name of schemaMethods) {
  if (!schemaMethodSchemas.has(name)) {
    problems.push(`\`${name}\` is named in the schema and has no \`params\`/\`result\` shape there`);
  }
}

console.log(
  `protocol: ${schemaMethods.size} methods named, ${schemaMethodSchemas.size} with a shape, ` +
    `${typeMethods.size} in types.ts, ${mapKeys.size} with params/result, ${dispatched.size} answered by the ` +
    `daemon, ${schemaEvents.size} events`,
);

if (problems.length > 0) {
  console.error('\nthe schema and protocol/types.ts disagree:');

  for (const problem of problems) {
    console.error(`  - ${problem}`);
  }

  process.exit(1);
}

console.log('protocol: schema, types.ts and the daemon dispatch agree');
