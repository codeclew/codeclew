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
