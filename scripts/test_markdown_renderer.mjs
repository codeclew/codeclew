import {test} from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,mkdir,readFile,writeFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {createHash} from 'node:crypto';
import {pathToFileURL} from 'node:url';
import {resolve} from 'node:path';
const rendererRoot=pathToFileURL(resolve(process.env.CODECLEW_MARKDOWN_RENDERER)+'/');
const {renderDirectory}=await import(new URL('render.mjs',rendererRoot));
const {validateNavigation}=await import(new URL('navigation.mjs',rendererRoot));
const {validateSequence,renderSequence}=await import(new URL('sequence.mjs',rendererRoot));

test('source footers keep local linked ranges and refuse changed excerpts, maps, index and Markdown',async()=>{
 const root=await mkdtemp(join(tmpdir(),'clew-source-footers-'));
 const sha=s=>'sha256:'+createHash('sha256').update(s).digest('hex');
 try{
  const input=join(root,'input');await mkdir(input);
  const body='# Reader\n\nBound text.\n',text='val example = "<literal>"\n';
  const source={service:'one',file:'src/OrderService.kt',revision:'rev-a',snapshot:'snapshot-a',startLine:40,endLine:40,textDigest:sha(text),url:null,repository:null,linkMapping:'MISSING'};
  const map=JSON.stringify({claims:[{sources:[source]}]}),index='{}';
  const bundle={schema:'codeclew-reader-sources/1',checkedIndex:join(root,'index.json'),indexFileDigest:sha(index),pages:[{
   page:'reader.md',markdownDigest:sha(body),map:join(root,'map.json'),mapFileDigest:sha(map),groups:[{
    service:source.service,file:source.file,revision:source.revision,snapshot:source.snapshot,ranges:[{startLine:40,endLine:40,text}]
   }]
  }]};
  const bundleFile=join(root,'sources.json');
  async function reset(){await writeFile(join(root,'index.json'),index);await writeFile(join(root,'map.json'),map);await writeFile(join(input,'reader.md'),body);await writeFile(bundleFile,JSON.stringify(bundle))}
  await reset();await renderDirectory(input,join(root,'good'),{sources:bundleFile,language:'ru'});
  const html=await readFile(join(root,'good/reader.html'),'utf8');
  assert.match(html,/<h2>\u0418\u0441\u0442\u043e\u0447\u043d\u0438\u043a\u0438<\/h2>/);
  assert.match(html,/href="#clew-source-0-0">40<\/a>/);assert.match(html,/id="clew-source-0-0"/);
  assert.match(html,/src\/OrderService.kt/);assert.match(html,/&lt;literal>/);assert.doesNotMatch(html,/<literal>|href="https?:/);
  const cases=[
   ()=>writeFile(join(root,'index.json'),'{"changed":true}'),
   ()=>writeFile(join(root,'map.json'),'{"claims":[]}'),
   ()=>writeFile(join(input,'reader.md'),body+'Changed'),
   ()=>writeFile(bundleFile,JSON.stringify({...bundle,pages:[{...bundle.pages[0],groups:[{...bundle.pages[0].groups[0],ranges:[{startLine:40,endLine:40,text:'Changed'}]}]}]}))
  ];
  for(const [i,change] of cases.entries()){
   await reset();await change();const output=join(root,'bad'+i);
   await assert.rejects(renderDirectory(input,output,{sources:bundleFile}),/Invalid source footer/);
   await assert.rejects(readFile(join(output,'reader.html')));
  }
 }finally{await rm(root,{recursive:true,force:true})}
});

const sequence=()=>({title:'Two entry branches',participants:[{id:'client',title:'Client'},{id:'handler',title:'Controller.post',target:'post.md#handler'},{id:'stock',title:'Stock API',target:'stock.md'}],steps:[
 {from:'client',to:'handler',kind:'call',label:'POST /v1/items',target:'post.md#handler'},
 {from:'handler',to:'stock',kind:'async',label:'Start a check',condition:'Flag enabled',target:'post.md#check'},
 {from:'handler',to:'stock',kind:'await',label:'Await the started check',condition:'Flag enabled'},
 {from:'stock',to:'handler',kind:'return',label:'Checked value',condition:'Check succeeds'},
 {from:'handler',to:'client',kind:'return',label:'200 result',condition:'Successful request'}]});

test('authored sequences render every step with explicit timing, conditions and local links',async()=>{
 const root=await mkdtemp(join(tmpdir(),'clew-sequence-'));
 try{
  const input=join(root,'input');await mkdir(input);
  await writeFile(join(input,'post.md'),'# Entry\n\n## Handler\n\n```clew-sequence\n'+JSON.stringify(sequence())+'\n```\n\n## Check\n\nBound explanation.\n');
  await writeFile(join(input,'stock.md'),'# Stock\n');
  for(const theme of ['codeclew','neutral']){
   const a=join(root,theme),b=join(root,theme+'-again');
   await renderDirectory(input,a,{theme});await renderDirectory(input,b,{theme});
   const html=await readFile(join(a,'post.html'),'utf8');assert.equal(html,await readFile(join(b,'post.html'),'utf8'));
   assert.doesNotMatch(html,/language-clew-sequence|<pre>/);assert.match(html,/<figure class="clew-sequence">/);
   assert.match(html,/class="sequence-message await"/);assert.match(html,/class="sequence-message return reverse"/);
   assert.match(html,/class="sequence-text" open/);assert.match(html,/href="stock.html"/);assert.match(html,/href="post.html#check"/);
   assert.match(html,/<div class="sequence-actors"><span>Client<\/span><span><a href="post.html#handler">Controller.post<\/a>/);
   assert.match(html,/<span>2\. <a href="post.html#check">Start a check<\/a>/);
   assert.match(html,/class="sequence-diagram" role="group" aria-label="Two entry branches"/);
   assert.doesNotMatch(html,/class="sequence-diagram" aria-hidden/);
   for(const step of sequence().steps){assert.ok(html.includes(step.label));if(step.condition)assert.ok(html.includes(step.condition))}
   assert.equal((html.match(/class="sequence-direction"/g)||[]).length,5);
   assert.match(html,/@media\(max-width:700px\).*sequence-scroll,.sequence-hint\{display:none\}/);
  }
  await renderDirectory(input,join(root,'ru'),{language:'ru'});
  const words=JSON.parse(await readFile(new URL('assets/ru.json',rendererRoot),'utf8'));
  assert.ok((await readFile(join(root,'ru/post.html'),'utf8')).includes(words.sequence.await));
 }finally{await rm(root,{recursive:true,force:true})}
});

test('sequence schema rejects ambiguity, invented kinds and broken destinations before writing',async()=>{
 for(const mutate of [v=>v.participants.push(v.participants[0]),v=>v.steps[0].from='missing',v=>v.steps[0].kind='ack',v=>v.steps[0].ordered=true,v=>v.steps[0].target='https://example.org/x.md',v=>v.steps[0].condition='']){
  const v=sequence();mutate(v);assert.throws(()=>validateSequence(v),/Invalid sequence/);
 }
 const root=await mkdtemp(join(tmpdir(),'clew-sequence-invalid-'));
 try{
  const input=join(root,'input');await mkdir(input);
  for(const bad of ['missing.md','post.md#absent']){
   const v=sequence();v.participants[1].target=bad;v.participants[2].target=undefined;v.steps.forEach(s=>delete s.target);
   await writeFile(join(input,'post.md'),'# Handler\n\n```clew-sequence\n'+JSON.stringify(v)+'\n```\n');
   await assert.rejects(renderDirectory(input,join(root,'output')),/page does not exist|anchor missing/);
   await assert.rejects(readFile(join(root,'output/post.html')));
  }
  await writeFile(join(input,'post.md'),'```clew-sequence\n{bad}\n```');
  await assert.rejects(renderDirectory(input,join(root,'bad-json')),/Invalid clew-sequence JSON/);
 }finally{await rm(root,{recursive:true,force:true})}
});

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
  const words=JSON.parse(await readFile(new URL('assets/ru.json',rendererRoot),'utf8'));
  assert.ok(html.includes(words.skip));assert.ok(html.includes(words.pages));assert.ok(html.includes(words.table));
  assert.match(html,/<html lang="ru">/);assert.match(html,/<title>Human field title — Project &quot;quoted&quot; &lt;label>/);
  assert.match(html,/<a href="fields.html" aria-current="page">Human field title<\/a>/);
  assert.match(html,/<table>\n<thead>/);assert.match(html,/<td><code>id<\/code><\/td>/);
  assert.match(html,/id="repeat-1"/);assert.ok(html.includes(component));
  assert.match(html,/href="favicon.svg"/);assert.deepEqual(await readFile(join(a,'favicon.svg')),await readFile(new URL('assets/favicon.svg',rendererRoot)));
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

function fixtureNavigation(){return {
 schema:'codeclew-markdown-navigation/1',home:{title:'Atlas',target:'welcome.md'},
 services:[{id:'bridge',title:'Bridge <one>',overview:'hub.md',pages:['hub.md','work.md','data.md','notes.md'],starts:[{title:'Accept input',kind:'http',target:'work.md#incoming',summary:'Stores the accepted request.'},{title:'Sweep pending',kind:'scheduled',target:'work.md#sweep',summary:'Processes pending records.'}],data:[{title:'Record fields',target:'data.md'}]}],
 routes:[{id:'journey',title:'Shared journey',target:'journey.md#entry',origin:{service:'bridge',title:'Accept input',kind:'http',target:'work.md#incoming'},participants:[{service:'bridge',title:'Intake role',role:'Receives <request>.',target:'work.md#incoming',section:'journey.md#entry'},{service:'bridge',title:'Background role',role:'Processes the record.',target:'work.md#sweep',section:'journey.md#entry'}]}]
}}
const navigationPages=[
 {file:'welcome.md',html:'<h1 id="atlas">Atlas</h1>'},{file:'hub.md',html:'<h1>Bridge</h1><p>Purpose.</p>'},
 {file:'work.md',html:'<h2 id="incoming">Incoming</h2><h2 id="sweep">Sweep</h2>'},
 {file:'data.md',html:'<h1>Fields</h1>'},{file:'notes.md',html:'<h1>Notes</h1>'},
 {file:'journey.md',html:'<h2 id="entry">Entry</h2>'},{file:'index.md',html:'<h1>Appendix</h1>'}
];

test('explicit generic navigation preserves content, roles, ownership and fallback destinations',async()=>{
 const root=await mkdtemp(join(tmpdir(),'clew-navigation-'));
 try{
  const input=join(root,'input');await mkdir(input);
  for(const p of navigationPages)await writeFile(join(input,p.file),p.html+'\n');
  const config=join(root,'navigation.json');await writeFile(config,JSON.stringify(fixtureNavigation()));
  const output=join(root,'output');await renderDirectory(input,output,{navigation:config});
  const hub=await readFile(join(output,'hub.html'),'utf8'),work=await readFile(join(output,'work.html'),'utf8'),route=await readFile(join(output,'journey.html'),'utf8');
  assert.match(hub,/<details class="service-navigation" open>/);
  assert.match(hub,/href="welcome.html"/);assert.match(hub,/Bridge &lt;one>/);
  assert.match(hub,/href="work.html#sweep"/);assert.match(hub,/Scheduled/);
  assert.match(hub,/href="notes.html"/);assert.match(hub,/href="index.html"/);
  assert.ok(hub.includes('<h1 id="bridge">Bridge</h1><p>Purpose.</p><section class="service-hub"'));
  assert.match(work,/<nav class="reader-breadcrumb"/);assert.ok(work.includes(navigationPages[2].html));
  assert.match(route,/Receives &lt;request>/);assert.match(route,/Intake role/);assert.match(route,/Background role/);
  const legacy=join(root,'legacy');await renderDirectory(input,legacy);
  assert.doesNotMatch(await readFile(join(legacy,'hub.html'),'utf8'),/<nav class="reader-breadcrumb"|<section class="service-hub"/);
  const neutral=join(root,'neutral');await renderDirectory(input,neutral,{navigation:config,theme:'neutral'});
  assert.doesNotMatch(await readFile(join(neutral,'hub.html'),'utf8'),/<nav class="reader-breadcrumb"|<section class="service-hub"/);
  const sparse=fixtureNavigation();sparse.services[0].starts=[];sparse.services[0].data=[];sparse.routes=[];
  const sparseOutput=join(root,'sparse');await renderDirectory(input,sparseOutput,{navigation:sparse});
  const sparseHtml=await readFile(join(sparseOutput,'hub.html'),'utf8');assert.doesNotMatch(sparseHtml,/<section class="service-hub"/);assert.match(sparseHtml,/href="work.html"/);
 }finally{await rm(root,{recursive:true,force:true})}
});

test('invalid navigation refuses missing targets, wrong ownership and missing origin participation',()=>{
 const cases=[
  [v=>v.services[0].starts[0].target='missing.md','page does not exist'],
  [v=>v.services[0].starts[0].target='work.md#absent','anchor missing'],
  [v=>v.services[0].starts[0].target='https://example.org/x.md','flat Markdown'],
  [v=>v.services[0].data[0].target='journey.md#entry','owned page'],
  [v=>v.routes[0].participants[0].service='unknown','unknown service'],
  [v=>v.routes[0].participants=[],'origin service must also be represented'],
  [v=>v.services[0].pages.push('work.md'),'duplicate page ownership'],
  [v=>v.services[0].starts[0].kind='imagined','expected http'],
  [v=>v.routes[0].participants[0].section='work.md#incoming','route page'],
  [v=>v.unexpected=true,'unknown property']
 ];
 for(const [change,message] of cases){const value=fixtureNavigation();change(value);assert.throws(()=>validateNavigation(value,navigationPages),new RegExp(message))}
 const value=fixtureNavigation();assert.equal(validateNavigation(value,navigationPages).routes[0].participants.length,2);
 const spaced=fixtureNavigation();spaced.services[0].pages[3]='field notes.md';
 const spacedPages=navigationPages.map(p=>p.file==='notes.md'?{...p,file:'field notes.md'}:p);
 assert.equal(validateNavigation(spaced,spacedPages).owners.get('field notes.md'),'bridge');
 const falseAnchor=navigationPages.map(p=>p.file==='work.md'?{...p,html:'<div data-id="incoming"></div><p> id="incoming"</p>'}:p);
 assert.throws(()=>validateNavigation(fixtureNavigation(),falseAnchor),/anchor missing/);
});
