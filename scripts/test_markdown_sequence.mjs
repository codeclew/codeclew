import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {renderSequence} from '../skills/codeclew/examples/markdown-renderer/sequence.mjs';
const fixture=JSON.parse(await readFile(new URL('./fixtures/human-http-reader/self-steps.json',import.meta.url),'utf8'));

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
 const html=renderSequence(value);
 assert.match(html,/<span>2\. <a href="worker.html#handler">404 response<\/a>/);
 assert.match(html,/· Local step<\/span><a href="worker.html#handler">404 response<\/a><p>Status remains 404<\/p>/);
 assert.match(html,/<span>3\. Wait<small>/);
 assert.match(html,/<span>4\. \u041f\u0440\u043e\u0432\u0435\u0440\u043a\u0430<small>/);
 assert.match(html,/<span>5\. 404\. Not Found<small>/);
 assert.match(html,/<span>6\. 2\.5 seconds<small>/);
 assert.match(html,/<span>7\. 10 records<small>/);
 assert.match(html,/<span>31\. Check result<small>/);
 assert.doesNotMatch(html,/1\. 404 response|31\. 30\.|Step 2:|\u0428\u0430\u0433 3:/);
 assert.equal(JSON.stringify(value),original);
});
