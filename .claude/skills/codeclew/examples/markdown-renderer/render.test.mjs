import {test} from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,mkdir,readFile,writeFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {renderDirectory} from './render.mjs';

test('Codeclew pages retain semantic tables, titles, stable links and inline components',async()=>{
 const root=await mkdtemp(join(tmpdir(),'clew-markdown-'));
 try{
  const input=join(root,'input');await mkdir(input);
  const component='<section id="original"><table><tr><td>Exact &amp; original</td></tr></table></section>';
  await writeFile(join(input,'index.md'),'# Home\n\n[Fields](fields.html#repeat-1)\n');
  await writeFile(join(input,'fields.md'),'# Human field title\n\n| Field | Meaning |\n| --- | --- |\n| `id` | Identity |\n\n## Repeat\n\n## Repeat\n\n'+component+'\n');
  const a=join(root,'one'),b=join(root,'two');
  await renderDirectory(input,a,{label:'Project "quoted" <label>',language:'ru'});
  await renderDirectory(input,b,{label:'Project "quoted" <label>',language:'ru'});
  for(const name of ['index.html','fields.html','favicon.svg'])assert.deepEqual(await readFile(join(a,name)),await readFile(join(b,name)));
  const html=await readFile(join(a,'fields.html'),'utf8');
  const words=JSON.parse(await readFile(new URL('./assets/ru.json',import.meta.url),'utf8'));
  assert.ok(html.includes(words.skip));assert.ok(html.includes(words.pages));assert.ok(html.includes(words.table));
  assert.match(html,/<html lang="ru">/);assert.match(html,/<title>Human field title — Project &quot;quoted&quot; &lt;label>/);
  assert.match(html,/<a href="fields.html" aria-current="page">Human field title<\/a>/);
  assert.match(html,/<table>\n<thead>/);assert.match(html,/<td><code>id<\/code><\/td>/);
  assert.match(html,/id="repeat-1"/);assert.ok(html.includes(component));
  assert.match(html,/href="favicon.svg"/);assert.deepEqual(await readFile(join(a,'favicon.svg')),await readFile(new URL('./assets/favicon.svg',import.meta.url)));
  assert.equal((html.match(/<svg aria-hidden="true" focusable="false"/g)||[]).length,2);
  assert.doesNotMatch(html,/<(?:input|button)\b/);assert.doesNotMatch(html,/\b(?:href|src)="(?:data:|https?:)/);
 }finally{await rm(root,{recursive:true,force:true})}
});

test('neutral option retains the previous ordinary reader and route rewriting',async()=>{
 const root=await mkdtemp(join(tmpdir(),'clew-neutral-'));
 try{
  const input=join(root,'input');await mkdir(input);await writeFile(join(input,'one.md'),'# One\n\n[Again](one.md#one)\n');
  const output=join(root,'output');await renderDirectory(input,output,{label:'Comparison',theme:'neutral'});
  const html=await readFile(join(output,'one.html'),'utf8');assert.match(html,/<title>Comparison<\/title>/);
  assert.match(html,/href="one.html#one"/);assert.match(html,/>one<\/a>/);assert.doesNotMatch(html,/reader-brand|favicon.svg/);
  await assert.rejects(renderDirectory(input,join(root,'bad'),{theme:'unrecognized'}));
 }finally{await rm(root,{recursive:true,force:true})}
});
