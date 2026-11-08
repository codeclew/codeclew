import {test} from 'node:test';
import assert from 'node:assert/strict';
import {renderSequence} from './sequence.mjs';
test('portable renderer sequence smoke',()=>{
 const v={title:'Preview',participants:[{id:'a',title:'Client'},{id:'b',title:'Handler'}],steps:[{from:'a',to:'b',kind:'call',label:'Request'}]};
 assert.match(renderSequence(v),/class="clew-sequence"/);
 assert.throws(()=>renderSequence({...v,steps:[]}),/Invalid sequence/);
});
