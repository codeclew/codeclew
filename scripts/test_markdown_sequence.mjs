import {test} from 'node:test';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {readFile} from 'node:fs/promises';
import {drawSequence,renderSequence} from '../skills/codeclew/examples/markdown-renderer/sequence.mjs';
const fixture=JSON.parse(await readFile(new URL('./fixtures/human-http-reader/self-steps.json',import.meta.url),'utf8'));

test('stock figure markup and CSS match the v0.15.6 bundled renderer',async()=>{
 const sha=value=>createHash('sha256').update(value).digest('hex');
 const css=await readFile(new URL('../skills/codeclew/examples/markdown-renderer/assets/sequence.css',import.meta.url));
 // The tagged renderer at 848f3c813519 produced this fixture output.
 assert.equal(sha(drawSequence(fixture)),'24782752b398fe55c616d151f6662da19b1bf518b09a9f9dbd83474be48f7c34');
 assert.equal(sha(css),'73116d5b1e5244a9f3f037f3bbb2d55f464728be403677d2771f9d7a5bcbf914');
});

test('self notes remain annotations while self calls retain loops and ordinary messages',()=>{
 const html=renderSequence(fixture);
 assert.equal((html.match(/class="sequence-loop"/g)||[]).length,2);
 assert.equal((html.match(/class="sequence-message note self"/g)||[]).length,2);
 assert.equal((html.match(/class="sequence-message call self"/g)||[]).length,2);
 assert.equal((html.match(/class="sequence-direction"/g)||[]).length,fixture.steps.length);
 assert.doesNotMatch(html,/label-left|translateX/);
 assert.match(html,/sequence-message return reverse/);assert.match(html,/sequence-message async/);
 for(const step of fixture.steps){assert.ok(html.includes(step.label));if(step.condition)assert.ok(html.includes(step.condition))}
 assert.match(html,/<a href="self-steps.html#details">Apply the selected calculation rule<\/a>/);
});
test('many actors, long labels and middle/end self steps retain text without synthesizing returns',()=>{
 const v=structuredClone(fixture);v.participants.push(...Array.from({length:8},(_,i)=>({id:'p'+i,title:'External participant '+i})));
 v.steps=Array.from({length:35},(_,i)=>({from:i%2?'calculation':'external',to:i%2?'calculation':'client',kind:i%2?'note':'call',label:i+' '+('Long <literal> & label '.repeat(12)),condition:'Explicit alternative '+i}));
 const html=renderSequence(v);
 assert.equal((html.match(/class="sequence-direction"/g)||[]).length,35);
 assert.equal((html.match(/class="sequence-row"/g)||[]).length,35);
 assert.match(html,/sequence-message note self/);assert.match(html,/34 Long &lt;literal> &amp; label/);
 assert.doesNotMatch(html,/<literal>|sequence-message return|NaN|undefined/);
});

test('aligned step ordinals are shown once in the diagram and ordered text trace',()=>{
 const target='worker.md#handler';
 const opening={from:'caller',to:'worker',kind:'async',label:'Timer fires',target};
 const value={title:'Background run',participants:[{id:'caller',title:'Timer'},{id:'worker',title:'Worker',target}],steps:[opening]};
 const step=label=>({from:'worker',to:'worker',kind:'note',label});
 value.steps.push({...step('1. 404 response'),target,condition:'Status remains 404'},step('Step 2: Wait'),
  step('\u0428\u0430\u0433 3: \u041f\u0440\u043e\u0432\u0435\u0440\u043a\u0430'),step('404. Not Found'),step('2.5 seconds'),step('10 records'));
 for(let position=8;position<31;position++)value.steps.push(step('Read records'));
 value.steps.push(step('30. Check result'));
 const original=JSON.stringify(value);
 const html=renderSequence(value,undefined,undefined,{id:'clew-sequence'});
 assert.match(html,/<a href="#clew-sequence-step-2-detail">2\. Work<\/a>/);
 assert.match(html,/· Local step<\/span><a href="worker.html#handler">404 response<\/a><p>Status remains 404<\/p>/);
 // Consecutive unconditioned local annotations share a number. Their detailed
 // labels retain values and authored text, with aligned ordinals removed.
 assert.match(html,/<li id="clew-sequence-step-3">[\s\S]*?Wait[\s\S]*?404\. Not Found[\s\S]*?2\.5 seconds[\s\S]*?10 records[\s\S]*?Check result/);
 assert.doesNotMatch(html,/1\. 404 response|31\. 30\.|Step 2:|\u0428\u0430\u0433 3:/);
 assert.equal(JSON.stringify(value),original);
});

test('projected systems keep source branches, a required wait and an independent read addressable',()=>{
 const local={id:'fulfillment',title:'Fulfillment',target:'overview.md'};
 const participants=[
  {id:'caller',title:'Client'},{id:'handler',title:'Handler',target:'submit.md#handler'},
  {id:'repo',title:'StorageOps'},{id:'inner',title:'StorageOps.Save'},
  {id:'host',title:'Fulfillment host'},{id:'worker',title:'PendingWorker'},
  {id:'store',title:'Database'},{id:'peer',title:'Unresolved peer'},
 ];
 const item=(from,to,kind,label,condition)=>({from,to,kind,label,condition});
 const steps=[
  {...item('caller','handler','call','POST /items'),target:'submit.md#handler'},
  item('handler','repo','call','Open the local repository'),
  item('repo','store','call','Execute `SELECT id FROM work_queue`'),
  item('repo','store','await','Await the lookup'),
  item('handler','caller','return','Return HTTP 404', 'Missing item'),
  item('handler','repo','call','Call the selected writer', 'Item exists'),
  item('repo','inner','call','Call Save'),
  item('inner','store','call','Execute `INSERT INTO work_queue(id) VALUES(id)`'),
  item('repo','store','call','Call `transaction.CommitAsync(token)`'),
  item('repo','store','await','Await commit completion before `Save` completes'),
  item('handler','repo','await','Await `Save` completion','Item exists'),
  item('handler','caller','return','Return HTTP 202','Item exists'),
  item('handler','handler','note','Throw after provider error','Commit fails'),
  item('host','worker','note','Host registers `AddHostedService<PendingWorker>()`'),
  item('worker','store','call','Select a row from `work_queue`'),
  item('worker','peer','async','Deliver when an active consumer receives the event','Delivery remains open'),
 ];
 const v={title:'Accept fulfillment',participants,steps:steps.map(s=>Object.fromEntries(Object.entries(s).filter(([,x])=>x!==undefined)))};
 const original=JSON.stringify(v);
 const options={id:'clew-sequence',systems:new Map([['submit.md',local]]),names:new Map([['fulfillment',local]]),
  sources:[
   {service:'fulfillment',file:'StorageOps.cs',ranges:[{startLine:1,text:'class StorageOps {\n public void Save() {\n  transaction.CommitAsync(token);\n }\n}'}]},
   {service:'fulfillment',file:'PendingWorker.cs',ranges:[{startLine:1,text:'class PendingWorker { }'}]},
  ]};
 const html=renderSequence(v,undefined,undefined,options);
 const figures=[...html.matchAll(/<figure class="clew-sequence">([\s\S]*?)<\/figure>/g)].map(row=>row[1]);
 assert.equal(figures.length,2);
 for(const figure of figures){
  assert.match(figure,/<p class="sequence-participants">/);
  assert.match(figure,/<\/div><\/div><\/div><details class="sequence-text" open>/);
 }
 const firstCount=[...figures[0].matchAll(/class="sequence-row"/g)].length;
 assert.ok(figures[1].includes('<ol start="'+(firstCount+1)+'">'));
 const diagrams=[...html.matchAll(/class="sequence-actors">(.*?)<\/div>/g)].map(row=>row[1]);
 assert.equal(diagrams.length,2);
 assert.match(diagrams[0],/Client.*Fulfillment.*Database/);
 assert.doesNotMatch(diagrams[0],/StorageOps|Unresolved peer/);
 assert.match(diagrams[1],/Fulfillment.*Database.*Unresolved peer/);
 assert.doesNotMatch(diagrams[1],/Client/);
 for(const name of ['Read · Wait','404 Not found','Insert','Commit · Wait','202 Accepted','Independent start'])
  assert.ok(html.includes(name),name);
 for(const step of v.steps)assert.ok(html.includes(step.label.replaceAll('<','&lt;')),step.label);
 assert.match(html,/The successful result on this path requires this wait to complete/);
 assert.match(html,/reader can see an earlier write when it becomes visible/);
 assert.doesNotMatch(html,/HTTP 500|HTTP 503/);
 assert.match(html,/data-clew-step-source="0-0"/);
 assert.equal([...html.matchAll(/class="sequence-direction"/g)].length,v.steps.length);
 assert.ok([...html.matchAll(/class="sequence-row"/g)].length<v.steps.length);
 for(const [,anchor] of html.matchAll(/href="#(clew-sequence-step-\d+)-detail"/g)){
  assert.equal(html.split('id="'+anchor+'"').length,2);
  assert.equal(html.split('id="'+anchor+'-detail" tabindex="-1"').length,2);
 }
 assert.equal(JSON.stringify(v),original);
 v.steps[10].condition='Different predicate';
 assert.doesNotMatch(renderSequence(v,undefined,undefined,options),/The successful result on this path requires/);
});

test('direct source links require a unique operation under the sending actor',()=>{
 const service={id:'work',title:'Work service'};
 const actors=[
  {id:'caller',title:'Caller'},{id:'writer',title:'Writer'},{id:'reader',title:'Tasks.Worker.Read'},
  {id:'judge',title:'evaluate'},{id:'other',title:'Foreign.Worker.Read'},
  {id:'db',title:'Database'},{id:'topic',title:'Broker topic'},{id:'peer',title:'Remote consumer'},
 ];
 const sql='SELECT id FROM work_queue ORDER BY created_at LIMIT 1 FOR UPDATE SKIP LOCKED';
 const rows=[
  ['caller','reader','call','Start work'],
  ['writer','db','call','Execute `INSERT INTO work_queue(id) VALUES(id)`'],
  ['reader','db','call','Select one `work_queue` row with `ORDER BY created_at LIMIT 1 FOR UPDATE SKIP LOCKED`'],
  ['reader','db','await','Await `reader.ReadAsync(token)`'],
  ['reader','db','call','Execute `DELETE FROM work_queue WHERE id=1`'],
  ['reader','db','call','Select an item in `work_queue`'],
  ['judge','reader','call','Read the evaluated item'],
  ['other','db','call','Execute `'+sql+'`'],
  ['topic','peer','async','Possible handoff on `queue.ready`'],
 ];
 const sequence={title:'Select work',participants:actors,steps:rows.map(([from,to,kind,label])=>({from,to,kind,label}))};
 const sources=[
  {service:'work',file:'Writer.cs',ranges:[{startLine:1,text:'class Writer {\n public void Write() {\n  command("INSERT INTO work_queue(id) VALUES(id)");\n  command("DELETE FROM work_queue WHERE id=1");\n  await reader.ReadAsync(token);\n }\n}'}]},
  {service:'work',file:'Worker.cs',ranges:[
   {startLine:1,text:'namespace Tasks;\nclass Worker { }'},
   {startLine:4,text:'public async Task Read() {\n command("'+sql+'");\n await reader.ReadAsync(token);\n}'},
  ]},
  {service:'work',file:'evaluate.ts',ranges:[{startLine:1,text:"export function evaluate() { return 'queue.ready'; }"}]},
  {service:'peer',file:'main.ts',ranges:[{startLine:1,text:"consumer.subscribe({ topics: ['queue.ready'] });"}]},
 ];
 const options={names:new Map([['work',service]]),sources};
 const render=()=>renderSequence(sequence,undefined,undefined,options);
 const anchors=html=>[...html.matchAll(/data-clew-step-source="([\d-]+)"/g)].map(m=>m[1]);
 const html=render();
 assert.deepEqual(anchors(html),['0-0','1-1','1-1']);
 const lanes=html.match(/class="sequence-actors">(.*?)<\/div>/)[1];
 assert.match(lanes,/Work service.*Foreign.Worker.Read.*Database.*Broker topic.*Remote consumer/);
 assert.doesNotMatch(lanes,/Tasks.Worker.Read|evaluate|Writer/);
 sources[1].ranges.push({startLine:20,text:sources[1].ranges[1].text});
 assert.deepEqual(anchors(render()),['0-0']);
});
