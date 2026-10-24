const {test}=require('node:test');
const assert=require('node:assert/strict');
const md=require('../crates/clew/assets/documentation/markdown.js');

test('pipe table preserves three columns, code, escaped pipes, links, alignment and empty cells',()=>{
 const html=md.render('Before.\n\nField | Name | Comment\n:--- | :---: | ---:\n`cityId` | City key | [Source](../source.html#id)\na\\|b | `x|y` | \n\nAfter.');
 assert.equal((html.match(/<th scope="col"/g)||[]).length,3);
 assert.equal((html.match(/<td/g)||[]).length,6);
 for(const text of ['<code>cityId</code>','a|b','<code>x|y</code>','href="../source.html#id"','text-align:center','<p>Before.</p>','<p>After.</p>'])assert.ok(html.includes(text),text);
});
test('optional outer pipes, empty cells and missing trailing cell remain intact',()=>{
 const html=md.render('| A | B | C |\n| --- | --- | --- |\n|  | two | |\n| one | two |');
 assert.equal((html.match(/<td/g)||[]).length,6);assert.match(html,/<td[^>]*><\/td><td[^>]*>two<\/td><td[^>]*><\/td>/);
 assert.ok(html.includes('data-label="B"'));
});
test('normal prose and malformed candidates are not tables or dropped text',()=>{
 for(const text of ['a | b','a | b\n-- | ---','a | b\n--- | ---\nx | y | KEEP EXTRA']){
  const html=md.render(text);assert.ok(!html.includes('<table'));assert.ok(html.includes(text.includes('KEEP EXTRA')?'KEEP EXTRA':'a | b'));
 }
});
test('fenced code and exact source-like literals do not become tables',()=>{
 const html=md.render('```text\nA | B\n--- | ---\n<script>literal</script>\n```');
 assert.ok(!html.includes('<table'));assert.ok(html.includes('<pre><code>A | B\n--- | ---\n&lt;script&gt;literal&lt;/script&gt;</code></pre>'));
 const indented=md.render('    | A | B |\n    | --- | --- |\n    | literal | text |');
 assert.ok(!indented.includes('<table'));assert.ok(indented.startsWith('<pre><code>| A | B |'));
});
test('raw HTML and unsafe URLs remain visible escaped text without active markup',()=>{
 const html=md.render('| A | B |\n| --- | --- |\n| <img src=x onerror=alert(1)> | [unsafe](javascript:alert) |\n| [data](data:text/html,x) | [safe](https://example.test/a?q=1&b=2) |');
 assert.ok(!html.includes('<img'));assert.ok(!html.includes('href="javascript:'));assert.ok(!html.includes('href="data:'));
 assert.ok(html.includes('&lt;img'));assert.ok(html.includes('href="https://example.test/a?q=1&amp;b=2"'));
 assert.ok(!md.inline('[bad](//example.test)').includes('<a'));
 assert.ok(!md.inline('[bad](java\\script:alert)').includes('<a'));
});
test('inline code, emphasis and literal escaping stay safe',()=>{
 assert.equal(md.inline('Use `a<b` and **clear** *names*.'),'Use <code>a&lt;b</code> and <strong>clear</strong> <em>names</em>.');
 assert.ok(md.render('| A | B |\n| --- | --- |\n| ``a`b|c`` | end |').includes('<code>a`b|c</code>'));
});
test('field links and initial hashes reveal technical disclosures without duplicate handlers',()=>{
 let focused=0;
 const events={},windowEvents={},details={tagName:'DETAILS',open:false,parentElement:null,querySelector:()=>({focus:()=>focused++})};
 const heading={tagName:'H3',nextElementSibling:details,parentElement:null};
 const document={querySelectorAll:()=>[],getElementById:id=>id==='field-name'?heading:null,
  addEventListener:(kind,handler)=>{assert.ok(!events[kind]);events[kind]=handler;},
  defaultView:{location:{hash:'#field-name'},addEventListener:(kind,handler)=>{assert.ok(!windowEvents[kind]);windowEvents[kind]=handler;}}};
 md.enhance(document);assert.equal(details.open,true);assert.equal(focused,0);
 details.open=false;events.click({target:{closest:()=>({hash:'#field-name'})}});assert.equal(details.open,true);assert.equal(focused,1);
 windowEvents.hashchange();assert.equal(focused,2);
 events.click({ctrlKey:true,target:{closest:()=>({hash:'#field-name'})}});assert.equal(focused,2);
 md.enhance(document);document.defaultView.location.hash='#%invalid';assert.doesNotThrow(()=>windowEvents.hashchange());
});
test('native field anchors reveal only the selected disclosure, preserving adjacent fields',()=>{
 let focused=0;
 const sibling={tagName:'DETAILS',open:false};
 const details={tagName:'DETAILS',open:false,parentElement:null,nextElementSibling:sibling,querySelector:()=>({focus:()=>focused++})};
 const events={};
 const document={querySelectorAll:()=>[],getElementById:id=>id==='field-latitude'?details:null,
  addEventListener:(kind,handler)=>{events[kind]=handler;},
  defaultView:{location:{hash:'#field-latitude'},addEventListener:()=>{}}};
 md.enhance(document);assert.equal(details.open,true);assert.equal(sibling.open,false);
 details.open=false;events.click({target:{closest:()=>({hash:'#field-latitude'})}});
 assert.equal(details.open,true);assert.equal(sibling.open,false);assert.equal(focused,1);
});

test('field hash scroll happens after disclosure layout and also on a later page enhancement',()=>{
 const scheduled=[],positions=[],events={};
 const details={tagName:'DETAILS',open:false,parentElement:null};
 const target={tagName:'H3',parentElement:details,scrollIntoView:()=>positions.push(details.open)};
 const document={querySelectorAll:()=>[],getElementById:id=>id==='field-saved'?target:null,
  addEventListener:(kind,handler)=>{assert.ok(!events[kind]);events[kind]=handler;},
  defaultView:{location:{hash:'#field-saved'},requestAnimationFrame:f=>scheduled.push(f),addEventListener:()=>{}}};
 md.enhance(document);assert.equal(details.open,true);assert.equal(positions.length,0);
 scheduled.shift()();assert.deepEqual(positions,[true]);
 details.open=false;md.enhance(document);scheduled.shift()();assert.deepEqual(positions,[true,true]);
});
