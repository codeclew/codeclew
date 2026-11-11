import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFile,mkdtemp,mkdir,writeFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {pathToFileURL} from 'node:url';
import {validateDocsetRoutes} from '../skills/codeclew/examples/markdown-renderer/docset-routes.mjs';
import {validateNavigation,navigationView} from '../skills/codeclew/examples/markdown-renderer/navigation.mjs';
import {docsetView,routeWords} from '../skills/codeclew/examples/markdown-renderer/route-view.mjs';
const fixture=JSON.parse(await readFile(new URL('../skills/codeclew/examples/markdown-renderer/docset-semantic.example.json',import.meta.url),'utf8'));
function inputs(docset,value=fixture){
 const local=value.docsets.find(d=>d.id===docset),names=new Set(['index.md',...local.services.map(s=>s.id+'.md')]);
 for(const r of value.semanticCatalog.records)if(r.target.docset===docset)names.add(r.target.page.replace('.html','.md'));
 for(const r of value.routes)if(r.narrative.docset===docset)names.add(r.narrative.page.replace('.html','.md'));
 const pages=[...names].map(file=>({file,title:file,html:['payments','tracking','devices','orders','done','scope'].map(a=>`<h2 id="${a}">${a}</h2>`).join('')}));
 const services=local.services.map(s=>({id:s.id,title:s.title,overview:s.id+'.md',pages:[s.id+'.md',...value.semanticCatalog.records.filter(r=>r.scope?.docset===docset&&r.scope.service===s.id).map(r=>r.target.page.replace('.html','.md'))],starts:[],data:[]}));
 const raw={schema:'codeclew-markdown-navigation/1',home:{title:'Home',target:'index.md'},services,routes:[]};
 return {pages,raw,nav:validateNavigation(raw,pages)};
}
test('five independent services share one canonical external and keep scoped homonyms and representations',()=>{
 assert.equal(fixture.docsets.flatMap(d=>d.services).length,5);
 for(const docset of ['east','west','catalog']){
  const {nav}=inputs(docset),model=validateDocsetRoutes(fixture,nav,docset,'r1');
  assert.equal(model.semantic.records.filter(r=>r.id==='payments').length,1);
  assert.equal(model.semantic.records.find(r=>r.id==='payments').uses.length,3);
  assert.equal(model.semantic.records.find(r=>r.id==='archive').uses.length,0);
  assert.equal(model.semantic.records.find(r=>r.id==='device-dto').represents,'device');
 }
 const {nav}=inputs('catalog'),model=validateDocsetRoutes(fixture,nav,'catalog','r1');
 const page=docsetView(model,'payments.md');
 assert.match(page.hub,/east\/orders.html#payments/);assert.match(page.hub,/east\/billing.html#payments/);assert.match(page.hub,/west\/enrollment.html#payments/);
 const hub=docsetView(model,'index.md');assert.match(hub.hub,/External systems/);assert.match(hub.hub,/Terms/);assert.doesNotMatch(hub.hub,/route-filters|Filter processes/);
 assert.match(hub.extra.semanticLinks.join(''),/Order · Orders · West/);
 assert.match(docsetView(model,'device.md').hub,/Representations/);
 assert.match(docsetView(model,'device.md').hub,/east\/device-dto.html/);
 const words={services:'Services',routes:'Processes',otherPages:'Other',breadcrumb:'Location'};
 assert.match(navigationView(nav,'payments.md',words,page.extra).breadcrumb,/Shared catalog/);
 assert.doesNotMatch(navigationView(nav,'payments.md',words,page.extra).breadcrumb,/>Processes</);
 const east=inputs('east'),local=docsetView(validateDocsetRoutes(fixture,east.nav,'east','r1'),'orders.md');
 assert.match(local.hub,/catalog\/payments.html/);assert.doesNotMatch(local.hub,/catalog\/tracking.html/);
 assert.match(local.extra.semanticLinks.join(''),/Order · Orders · East/);
 assert.match(docsetView(validateDocsetRoutes(fixture,east.nav,'east','r1'),'device-dto.md').hub,/Representation of/);
});
test('explicit alias collisions, ownership, scoped uses, representation promotion and missing local fragments refuse',()=>{
 for(const change of [
  v=>v.semanticCatalog.records[1].aliases.push(v.semanticCatalog.records[0].aliases[0]),
  v=>v.semanticCatalog.records[0].target=v.semanticCatalog.records[1].target,
  v=>v.semanticCatalog.records[0].uses[0].target.anchor='absent',
  v=>{const u=structuredClone(v.semanticCatalog.records[0].uses[0]);u.target={page:u.target.page,anchor:u.target.anchor,docset:u.target.docset};v.semanticCatalog.records[0].uses.push(u)},
  v=>v.semanticCatalog.records[4].uses.push(v.semanticCatalog.records[5].uses[0]),
  v=>v.semanticCatalog.records[6].represents='payments',
  v=>v.externalActors=[{id:'bad',title:'Bad',canonical:'device'}],
  v=>v.semanticCatalog.records[0].target={docset:'east',page:'orders.html'},
  v=>v.schema='codeclew-cross-docset-routes/1.1'
 ]){
  const value=structuredClone(fixture);change(value);assert.throws(()=>validateDocsetRoutes(value,inputs('east').nav,'east','r1'));
 }
});
test('the supported renderer adds canonical backlinks without rewriting local authored Markdown',async()=>{
 const {renderDirectory}=await import(new URL('render.mjs',pathToFileURL(process.env.CODECLEW_MARKDOWN_RENDERER+'/')));
 const root=await mkdtemp(join(tmpdir(),'clew-semantic-'));
 try{
  for(const docset of ['east','west','catalog']){
   const {pages,raw}=inputs(docset),source=join(root,docset),out=join(root,docset+'-html');await mkdir(source);
   const texts=new Map(pages.map(p=>[p.file,`# ${p.file}\n\nOwned fixture.\n\n## Payments\n\n## Tracking\n\n## Devices\n\n## Orders\n`]));
   for(const [file,text] of texts)await writeFile(join(source,file),text);
   await renderDirectory(source,out,{navigation:raw,docsetRoutes:fixture,docset,docsetVersion:'r1'});
   for(const [file,text] of texts)assert.equal(await readFile(join(source,file),'utf8'),text);
   if(docset==='catalog'){
    const page=await readFile(join(out,'payments.html'),'utf8');assert.match(page,/Used by/);assert.match(page,/east\/billing.html#payments/);assert.doesNotMatch(page,/Filter processes/);
    const index=await readFile(join(out,'index.html'),'utf8');assert.match(index,/External systems/);assert.doesNotMatch(index,/<form class="route-filters"/);
   }else assert.match(await readFile(join(out,'orders.html'),'utf8'),new RegExp('catalog/'+(docset==='east'?'payments':'tracking')+'.html'));
  }
 }finally{await rm(root,{recursive:true,force:true})}
});
test('explicit canonical IDs connect external process participants and sequence arrows, with no name inference',async()=>{
 const {renderDirectory}=await import(new URL('render.mjs',pathToFileURL(process.env.CODECLEW_MARKDOWN_RENDERER+'/')));
 const value=structuredClone(fixture),target=(docset,page,anchor)=>({docset,page,...(anchor?{anchor}:{})});
 value.externalActors=[{id:'payment-system',title:'Payment system',canonical:'payments'}];
 value.routes=[{id:'charge',version:'r1',title:'Charge',narrative:target('catalog','charge.html'),origin:{status:'unknown',reason:'Fixture does not establish a trigger'},
  occurrences:[{id:'local',actor:{kind:'service',docset:'east',service:'orders'},role:'Request payment',target:target('east','orders.html','payments')},{id:'remote',actor:{kind:'external',id:'payment-system'},role:'Payment boundary',target:target('catalog','payments.html')}],
  branches:[{id:'selected',title:'Selected boundary',condition:'Declared fixture path',target:target('catalog','charge.html','done')}],
  outcomes:[{id:'handoff',branch:'selected',kind:'handoff',title:'Request sent',target:target('catalog','charge.html','done'),internal:{coverage:'unknown',occurrences:[]},end:{status:'known',ownerOccurrence:'local',boundary:'handoff',recipient:{kind:'external',id:'payment-system'}}}],
  coverage:{outcomeInventory:'partial',scope:'Only the declared fixture boundary',reference:target('catalog','charge.html','scope')}}];
 const root=await mkdtemp(join(tmpdir(),'clew-external-sequence-'));
 try{
  const {pages,raw}=inputs('catalog',value),source=join(root,'input');await mkdir(source);
  const sequence={title:'Owned external call',participants:[{id:'local',title:'Orders'},{id:'remote',title:'Payment system',catalogRef:'payments'}],steps:[{from:'local',to:'remote',label:'Payment boundary',kind:'call',catalogRef:'payments'}]};
  for(const p of pages)await writeFile(join(source,p.file),'# '+p.file+'\n\n## Done\n\n## Scope\n'+(p.file==='charge.md'?'\n```clew-sequence\n'+JSON.stringify(sequence)+'\n```\n':''));
  const options={navigation:raw,docsetRoutes:value,docset:'catalog',docsetVersion:'r1'};
  await renderDirectory(source,join(root,'output'),options);
  const html=await readFile(join(root,'output/charge.html'),'utf8');
  assert.match(html,/<a href="payments.html">Payment boundary<\/a>/);assert.match(html,/<a href="payments.html"[^>]*>Payment system<\/a>/);
  assert.doesNotMatch(html,/data-clew-catalog-ref/);
  sequence.steps[0].catalogRef='device';await writeFile(join(source,'charge.md'),'# Charge\n\n## Done\n\n## Scope\n\n```clew-sequence\n'+JSON.stringify(sequence)+'\n```\n');
  await assert.rejects(renderDirectory(source,join(root,'wrong-kind'),options),/explicit canonical external/);
  await assert.rejects(renderDirectory(source,join(root,'no-registry'),{navigation:raw}),/explicit canonical external/);
 }finally{await rm(root,{recursive:true,force:true})}
});
