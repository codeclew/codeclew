import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {validateDocsetRoutes,filterRoutes} from '../skills/codeclew/examples/markdown-renderer/docset-routes.mjs';
import {validateNavigation,navigationView} from '../skills/codeclew/examples/markdown-renderer/navigation.mjs';
const registry=JSON.parse(await readFile(new URL('../fixtures/docs-cross-docset-routes/registry.json',import.meta.url),'utf8'));
const cases=JSON.parse(await readFile(new URL('../fixtures/docs-cross-docset-routes/filter-cases.json',import.meta.url),'utf8'));
const clone=()=>structuredClone(registry);
function nav(docset){
 const defs={portal:{'index.md':['directory']},journeys:{'index.md':['journeys'],'stock-query.md':['stock','success','returned','scope'],'order-submit.md':['submit','completed','rejected','handoff','pending','scope']},'orders-docs':{'overview.md':['orders'],'incoming.md':['request','response'],'submit.md':['entry','callback','completed','publish']},'inventory-docs':{'overview.md':['inventory'],'handler.md':['lookup'],'reservation.md':['reserve']}};
 const pages=Object.entries(defs[docset]).map(([file,ids])=>({file,title:file,html:ids.map(id=>`<h2 id="${id}">${id}</h2>`).join('')}));
 const home=pages[0].file;
 const services=['orders-docs','inventory-docs'].includes(docset)?[{id:'api',title:docset,overview:home,pages:pages.map(p=>p.file),starts:docset==='orders-docs'?[{title:'Query',kind:'http',summary:'Query start',target:'incoming.md#request'},{title:'Submit',kind:'http',summary:'Submit start',target:'submit.md#entry'}]:[],data:[]}]:[];
 return validateNavigation({schema:'codeclew-markdown-navigation/1',home:{title:'Home',target:home},services,routes:[]},pages);
}
const version=id=>registry.docsets.find(d=>d.id===id).version;
test('eleven exact query results use a single compatible outcome',()=>{
 for(const row of cases.cases)assert.deepEqual(Object.fromEntries(filterRoutes(registry,row.query).map(r=>[r.id,r.status])),row.expected,row.id);
 const result=filterRoutes(registry,cases.cases.find(r=>r.id==='return-via-b').query)[1];assert.deepEqual(result.matches,[{branch:'completed',outcome:'returned'}]);
});
test('known start survives zero outcomes; unknown origin is never inferred from occurrence order',()=>{
 const v=clone(),r=v.routes[1];r.outcomes=[];r.coverage.outcomeInventory='unknown';
 assert.equal(filterRoutes(v,{start:[cases.identityKeys.A]})[1].status,'MATCH');
 assert.equal(filterRoutes(v,{end:[cases.identityKeys.A]})[1].status,'UNCLASSIFIED');
 r.origin={status:'unknown',reason:'Not classified'};
 assert.equal(filterRoutes(v,{start:[cases.identityKeys.A]})[1].status,'UNCLASSIFIED');
 assert.equal(filterRoutes(v)[1].status,'MATCH');
});
test('terminal owner differs from recipient and external actors are not service ends',()=>{
 const v=clone();v.routes=[v.routes[1]];v.routes[0].outcomes=[v.routes[0].outcomes[1]];v.routes[0].coverage.outcomeInventory='complete-for-declared-scope';
 assert.equal(filterRoutes(v,{end:[cases.identityKeys.A]})[0].status,'NO_MATCH');
 assert.equal(filterRoutes(v,{end:[cases.identityKeys.C]})[0].status,'MATCH');
 v.routes[0].outcomes=[registry.routes[1].outcomes[2]];
 for(const actor of Object.values(cases.identityKeys))assert.equal(filterRoutes(v,{end:[actor]})[0].status,'NO_MATCH');
});
test('independent A/B/journeys/portal admissions do not need peer pages; repeated service roles survive',()=>{
 for(const id of ['orders-docs','inventory-docs','journeys','portal']){
  const model=validateDocsetRoutes(registry,nav(id),id,version(id));assert.equal(model.routes.length,2);
  assert.equal(model.routes[1].occurrences.filter(o=>o.actor.docset==='orders-docs').length,4);
 }
 const model=validateDocsetRoutes(registry,nav('orders-docs'),'orders-docs',version('orders-docs'));
 assert.equal(model.services.find(s=>s.docset==='orders-docs').to.kind,'LOCAL_DOCSET');
 assert.equal(model.services.find(s=>s.docset==='inventory-docs').to.kind,'REMOTE_DOCSET');
 assert.equal(model.services.find(s=>s.docset==='inventory-docs').to.href,'https://docs.example.invalid/inventory-docs/overview.html');
});
test('URL namespaces and path traversal are refused, including encoded and normalization aliases',()=>{
 const changes=[v=>v.docsets[1].baseURL='https://DOCS.example.invalid:443/orders-docs/',v=>v.docsets[1].baseURL='https://docs.example.invalid/orders-docs/nested/',...['../x.html','a/b.html','%2e%2e.html','%252e.html','C:\\x.html','file:x.html'].map(page=>v=>v.routes[0].narrative.page=page),...['https://docs.example.invalid/x/../y/','https://docs.example.invalid/%2e%2e/','https://docs.example.invalid/%252e/','//docs.example.invalid/x/','javascript:alert(1)','file:///tmp/','https://fixture-user@docs.example.invalid/x/','https://docs.example.invalid/x/?q=1','https://docs.example.invalid/x/#fragment'].map(url=>v=>v.docsets[0].baseURL=url)];
 for(const change of changes){const v=clone();change(v);assert.throws(()=>validateDocsetRoutes(v,nav('portal'),'portal',version('portal')))}
});
test('local owner, anchors, origin starts and versions remain required',()=>{
 for(const change of [v=>v.routes[0].occurrences[0].target.page='absent.html',v=>v.routes[0].occurrences[0].target.anchor='absent',v=>v.routes[0].origin.kind='kafka',v=>v.routes[0].occurrences[0].actor.service='missing',v=>v.routes[0].occurrences[0].target.docset='inventory-docs']){
  const v=clone();change(v);assert.throws(()=>validateDocsetRoutes(v,nav('orders-docs'),'orders-docs',version('orders-docs')));
 }
 assert.throws(()=>validateDocsetRoutes(registry,nav('portal'),'portal','wrong-version'));
 const local=nav('orders-docs');local.owners.set('incoming.md','other');assert.throws(()=>validateDocsetRoutes(registry,local,'orders-docs',version('orders-docs')));
 const shared=nav('journeys');shared.owners.set('stock-query.md','fake');assert.throws(()=>validateDocsetRoutes(registry,shared,'journeys',version('journeys')));
});
test('duplicate IDs, dangling references and conflicting occurrence roles are rejected',()=>{
 for(const change of [v=>v.routes[0].occurrences.push(v.routes[0].occurrences[0]),v=>v.routes[0].outcomes[0].branch='absent',v=>v.routes[0].origin.occurrence='absent',v=>v.routes[0].outcomes[0].internal.occurrences=['a-entry'],v=>v.routes[0].outcomes[0].internal.occurrences=['a-return'],v=>v.routes[0].outcomes[0].end.ownerOccurrence='missing',v=>v.routes.push(v.routes[0]),v=>v.docsets.push(v.docsets[0]),v=>v.routes[0].unexpected=true,v=>v.catalog=null,v=>v.routes[0].narrative.problem=null,v=>v.routes[0].narrative.problem=false]){
  const v=clone();change(v);assert.throws(()=>validateDocsetRoutes(v,nav('portal'),'portal',version('portal')));
 }
});
test('reported remote problems retain routes; no remote request establishes availability',()=>{
 const v=clone();v.routes[0].narrative.problem={kind:'reported-version-drift',note:'Owner reported an older target',reportedBy:'Fixture owner',observedVersion:'old'};
 const model=validateDocsetRoutes(v,nav('portal'),'portal',version('portal'));assert.equal(model.routes.length,2);assert.equal(model.routes[0].to.problem.kind,'reported-version-drift');
 assert.equal(filterRoutes(v)[0].status,'MATCH');
});
test('singleton label keeps local service title; two services retain plural group; zero has none',()=>{
 const words={services:'Services',overview:'Overview',starts:'Starts',data:'Data',participation:'Participation',routes:'Routes',otherPages:'Other',breadcrumb:'Location'};
 const single=nav('inventory-docs'),html=navigationView(single,'overview.md',words).menu;assert.doesNotMatch(html,/<h2[^>]*>Services/);assert.match(html,/<summary>inventory-docs/);
 const two={...single,services:[...single.services,{...single.services[0],id:'other',title:'Other'}]};assert.match(navigationView(two,'overview.md',words).menu,/<h2[^>]*>Services/);
 assert.doesNotMatch(navigationView(nav('portal'),'index.md',words).menu,/<h2[^>]*>Services/);
});

test('filter input rejects typos and external actors masquerading as service selections',()=>{
 assert.throws(()=>filterRoutes(registry,{ends:[]}),/filter property/);
 assert.throws(()=>filterRoutes(registry,{start:[{kind:'external',docset:'orders-docs',service:'api'}]}),/service filter/);
 const local=nav('orders-docs');local.byFile.get('incoming.md').html+='<span id="request"></span>';
 assert.throws(()=>validateDocsetRoutes(registry,local,'orders-docs',version('orders-docs')),/ambiguous/);
});

test('explicit facet values and typed service identities reject coercion',()=>{
 for(const field of ['start','end','intermediate','outcomeKinds']){
  for(const value of [false,null,0,'',undefined,{},'orders-docs/api'])assert.throws(()=>filterRoutes(registry,{[field]:value}),undefined,field+': '+String(value));
 }
 for(const field of ['start','end','intermediate']){
  for(const property of ['docset','service'])for(const value of [['orders-docs'],['api'],null,false,0,{},undefined]){
   const actor={kind:'service',docset:'orders-docs',service:'api',[property]:value};assert.throws(()=>filterRoutes(registry,{[field]:[actor]}),undefined,field+'/'+property);
  }
  assert.deepEqual(filterRoutes(registry,{[field]:[{kind:'service',docset:'orders-docs',service:'api'}]}),filterRoutes(registry,{[field]:['orders-docs/api']}));
 }
 assert.deepEqual(filterRoutes(registry),filterRoutes(registry,{start:[],end:[],intermediate:[],outcomeKinds:[]}));
});
test('registry service actors reject non-string namespace IDs before key construction',()=>{
 for(const position of ['occurrence','recipient'])for(const property of ['docset','service'])for(const value of [['orders-docs'],['api'],null,false,0,{},undefined]){
  const v=clone(),actor=position==='occurrence'?v.routes[1].occurrences[0].actor:v.routes[1].outcomes[1].end.recipient;
  assert.equal(actor.kind,'service');actor[property]=value;
  assert.throws(()=>validateDocsetRoutes(v,nav('portal'),'portal',version('portal')),undefined,position+'/'+property);
 }
});
