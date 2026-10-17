import {readFile, readdir, mkdir, rm, writeFile, rename, lstat} from 'node:fs/promises';
import {resolve, join} from 'node:path';
import {createHash} from 'node:crypto';

const root = resolve(process.argv[2] || process.cwd());
const data = join(root, 'static', 'clew-data');
const output = join(root, 'static', 'clew-reader-packs');
const maxPackBytes = 256 * 1024;
const objects = new Map();
function refs(value, result = new Set()) {
  if (!value || typeof value !== 'object') return result;
  if (!Array.isArray(value) && Object.keys(value).length === 1 && typeof value.$clewRef === 'string') {
    if (!/^sha256:[a-f0-9]{64}$/.test(value.$clewRef)) throw new Error('Invalid saved object identity');
    result.add(value.$clewRef); return result;
  }
  const literal = !Array.isArray(value) && Object.keys(value).length === 1 && Object.hasOwn(value, '$clewLiteral');
  for (const child of Object.values(literal ? value.$clewLiteral : value)) refs(child, result);
  return result;
}
async function object(id) {
  if (!objects.has(id)) {
    const raw = await readFile(join(data, 'objects', id.slice(7) + '.json'), 'utf8');
    const record = JSON.parse(raw);
    if (record.schema !== 'codeclew-export-object/1.0' || record.identity !== id) throw new Error('Saved object identity differs');
    objects.set(id, {raw, record});
  }
  return objects.get(id);
}
async function closure(ids, seen = new Set(), active = new Set()) {
  for (const id of ids) {
    if (active.has(id)) throw new Error('Cyclic saved object reference');
    if (seen.has(id)) continue;
    active.add(id);
    const saved = await object(id);
    await closure(refs(saved.record.value), seen, active);
    active.delete(id); seen.add(id);
  }
  return seen;
}
const widgets = new Map();
const widgetNames = await readdir(join(data, 'widgets')).catch(error => {if (error.code === 'ENOENT') return []; throw error;});
for (const name of widgetNames.filter(name => name.endsWith('.json')).sort()) {
  const saved = JSON.parse(await readFile(join(data, 'widgets', name), 'utf8'));
  if (saved.schema !== 'codeclew-normalized-data/1.0') throw new Error('Unsupported widget document');
  widgets.set('/clew-data/widgets/' + name, await closure(refs(saved.value)));
}
// Identical reader-use sets partition the objects: no object duplication and no
// unrelated-object overfetch. Packs are derived deployment data, never Git input.
const uses = new Map();
for (const [widget, ids] of widgets) for (const id of ids) {
  if (!uses.has(id)) uses.set(id, []);
  uses.get(id).push(widget);
}
const groups = new Map();
for (const [id, members] of [...uses].sort()) {
  const key = JSON.stringify(members);
  if (!groups.has(key)) groups.set(key, []);
  groups.get(key).push(id);
}
// Build all pack contents before replacing a prior generated directory.
const packs = new Map(), manifests = Object.fromEntries([...widgets].map(([name, ids]) => [name, {objects: ids.size, packs: []}]));
for (const [key, ids] of [...groups].sort()) {
  const members = JSON.parse(key), hash = createHash('sha256').update(key).digest('hex');
  let part = [], bytes = 0, index = 0;
  function flush() {
    if (!part.length) return;
    const name = `pack-${hash}-${index++}.json`;
    // Copy original JSON records verbatim, including numeric spelling and source
    // strings. Transport packing must not change their content-addressed value.
    const body = '{"schema":"codeclew-reader-pack/1.0","objects":{\n' + part.map(id => JSON.stringify(id) + ':' + objects.get(id).raw).join(',\n') + '\n}}\n';
    packs.set(name, body); for (const member of members) manifests[member].packs.push('/clew-reader-packs/' + name);
    part = []; bytes = 0;
  }
  for (const id of ids) {
    const size = Buffer.byteLength(objects.get(id).raw);
    if (part.length && bytes + size > maxPackBytes) flush();
    part.push(id); bytes += size;
  }
  flush();
}
let previous = null;
try {
  if (!(await lstat(output)).isDirectory()) throw new Error('Reader pack output must be a regular directory');
  previous = JSON.parse(await readFile(join(output, 'manifest.json'), 'utf8'));
  if (previous.schema !== 'codeclew-reader-packs/1.0') throw new Error('Unrecognized reader pack manifest');
} catch (error) {if (error.code !== 'ENOENT') throw error;}
await mkdir(output, {recursive: true});
async function replace(name, body) {
  const pending = join(output, '.' + name + '.pending');
  await writeFile(pending, body, {flag: 'wx'}); await rename(pending, join(output, name));
}
for (const [name, body] of packs) await replace(name, body);
await replace('manifest.json', JSON.stringify({schema: 'codeclew-reader-packs/1.0', widgets: manifests}, null, 2) + '\n');
for (const widget of Object.values(previous?.widgets || {})) for (const path of widget.packs) {
  const name = path.split('/').pop();
  if (/^pack-[a-f0-9]{64}-[0-9]+\.json$/.test(name) && !packs.has(name)) await rm(join(output, name), {force: true});
}
console.log(`Prepared ${packs.size} shared reader packs for ${widgets.size} field readers.`);
