import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFile,writeFile,mkdir,mkdtemp,rm,stat} from 'node:fs/promises';
import {join} from 'node:path';
import {tmpdir} from 'node:os';
import {pathToFileURL} from 'node:url';
import {createHash} from 'node:crypto';
const {renderDirectory}=await import(new URL('render.mjs',pathToFileURL(process.env.CODECLEW_MARKDOWN_RENDERER+'/')));
const registry=JSON.parse(await readFile(new URL('../fixtures/docs-cross-docset-routes/registry.json',import.meta.url),'utf8'));
const files={portal:{'index.md':'# Directory\n'},journeys:{'index.md':'# Journeys\n','stock-query.md':'# Stock\n\n## Success\n\n## Returned\n\n## Scope\n','order-submit.md':'# Submit\n\n## Completed\n\n## Rejected\n\n## Handoff\n\n## Pending\n\n## Scope\n'},'orders-docs':{'overview.md':'# Orders\n','incoming.md':'# Incoming\n\n## Request\n\n## Response\n','submit.md':'# Submit\n\n## Entry\n\n## Callback\n\n## Completed\n\n## Publish\n'},'inventory-docs':{'overview.md':'# Inventory\n','handler.md':'# Handler\n\n## Lookup\n','reservation.md':'# Reservation\n\n## Reserve\n'}};
function options(id,value=registry){
 const names=Object.keys(files[id]),services=id.endsWith('-docs')?[{id:'api',title:id,overview:names[0],pages:names,starts:id==='orders-docs'?[{title:'Query',target:'incoming.md#request',kind:'http',summary:'Start query'},{title:'Submit',target:'submit.md#entry',kind:'http',summary:'Start submit'}]:[],data:[]}]:[];
 return {docset:id,docsetVersion:registry.docsets.find(d=>d.id===id).version,docsetRoutes:value,navigation:{schema:'codeclew-markdown-navigation/1',home:{title:'Home',target:names[0]},services,routes:[]}};
}
async function input(root,id,name=id){const p=join(root,name);await mkdir(p);for(const [f,text] of Object.entries(files[id]))await writeFile(join(p,f),text);return p}
const sha=b=>'sha256:'+createHash('sha256').update(b).digest('hex');
test('independent renders expose scoped links, canonical return and registry identity without fetching peers',async()=>{
 const root=await mkdtemp(join(tmpdir(),'clew-docsets-'));const fetch=globalThis.fetch;globalThis.fetch=()=>{throw Error('Rendering must not fetch')};
 try{
  for(const id of ['orders-docs','inventory-docs','journeys','portal']){
   const source=await input(root,id),out=join(root,id+'-out');const result=await renderDirectory(source,out,options(id));
   assert.equal(result.crossDocsetRoutes.registryDigest,sha(JSON.stringify(registry)));assert.equal(result.crossDocsetRoutes.remoteTargets,'DECLARED_NOT_FETCHED');
   const page=await readFile(join(out,id==='journeys'?'order-submit.html':Object.keys(files[id])[0].replace('.md','.html')),'utf8');
   assert.ok(page.includes(result.crossDocsetRoutes.registryDigest));assert.match(page,/codeclew-docset-version/);
   if(id==='orders-docs')assert.match(page,/http[s]?:\/\/docs.example.invalid\/journeys\/order-submit.html/);
   if(id==='journeys')assert.match(page,/http[s]?:\/\/docs.example.invalid\/orders-docs\/overview.html/);
   if(id==='portal'){assert.match(page,/type="checkbox" name="start"/);assert.match(page,/Processes with classification gaps/);assert.match(page,/Ends in: Normal: Orders API/);assert.match(page,/route-count/)}
  }
 }finally{globalThis.fetch=fetch;await rm(root,{recursive:true,force:true})}
});
test('render bytes ignore temporary directories but include changes to otherwise unused registry revision',async()=>{
 const root=await mkdtemp(join(tmpdir(),'clew-docset-replay-'));
 try{
  const a=await input(root,'orders-docs','a'),b=await input(root,'orders-docs','b');
  await renderDirectory(a,join(root,'first'),options('orders-docs'));await renderDirectory(b,join(root,'second'),options('orders-docs'));
  const first=await readFile(join(root,'first/overview.html'),'utf8');assert.equal(first,await readFile(join(root,'second/overview.html'),'utf8'));
  const changed=structuredClone(registry);changed.revision='different-revision';await renderDirectory(b,join(root,'changed'),options('orders-docs',changed));
  const other=await readFile(join(root,'changed/overview.html'),'utf8');assert.notEqual(first,other);
  assert.equal(first.replace(/name="codeclew-registry-digest" content="[^"]+"/,''),other.replace(/name="codeclew-registry-digest" content="[^"]+"/,''));
 }finally{await rm(root,{recursive:true,force:true})}
});
test('registry byte bound accepts exactly 1 MiB, rejects +1 before output and does not raise local bounds',async()=>{
 const root=await mkdtemp(join(tmpdir(),'clew-registry-bound-'));
 try{
  const source=await input(root,'portal'),path=join(root,'registry.json'),raw=JSON.stringify(registry),limit=1024*1024;
  await writeFile(path,raw+' '.repeat(limit-Buffer.byteLength(raw)));await renderDirectory(source,join(root,'exact'),options('portal',path));
  await writeFile(path,raw+' '.repeat(limit+1-Buffer.byteLength(raw)));const output=join(root,'overflow');await assert.rejects(renderDirectory(source,output,options('portal',path)),/1 MiB/);await assert.rejects(stat(output));
  const bad=options('portal');delete bad.docsetVersion;await assert.rejects(renderDirectory(source,join(root,'incomplete'),bad),/require/);
 }finally{await rm(root,{recursive:true,force:true})}
});
test('unknown data renders safely and remote target problems keep a disabled process entry',async()=>{
 const root=await mkdtemp(join(tmpdir(),'clew-route-safe-'));
 try{
  const value=structuredClone(registry);value.routes[0].title='</script><img src=x onerror=alert(1)>';
  value.routes[0].narrative.problem={kind:'reported-missing',note:'Owner has not published this page',reportedBy:'Fixture'};
  const source=await input(root,'portal'),out=join(root,'out');await renderDirectory(source,out,options('portal',value));
  const html=await readFile(join(out,'index.html'),'utf8');assert.match(html,/route-link-unavailable/);assert.match(html,/&lt;\/script&gt;&lt;img/);assert.doesNotMatch(html,/<img src=x/);
  assert.equal((html.match(/data-route-id=/g)||[]).length,2);
  for(const [,type,script] of html.matchAll(/<script(?: type="([^"]+)")?(?: id="[^"]+")?>([\s\S]*?)<\/script>/g))if(type!=='application/json')assert.doesNotThrow(()=>new Function(script));
 }finally{await rm(root,{recursive:true,force:true})}
});

test('Russian filter chrome keeps author labels and explicit uncertainty',async()=>{
 const root=await mkdtemp(join(tmpdir(),'clew-route-ru-'));
 try{
  const source=await input(root,'portal'),out=join(root,'out');await renderDirectory(source,out,{...options('portal'),language:'ru'});
  const html=await readFile(join(out,'index.html'),'utf8');assert.match(html,/\u0424\u0438\u043b\u044c\u0442\u0440\u044b \u043f\u0440\u043e\u0446\u0435\u0441\u0441\u043e\u0432/);assert.match(html,/\u041f\u0440\u043e\u0446\u0435\u0441\u0441\u044b \u0441 \u043f\u0440\u043e\u0431\u0435\u043b\u0430\u043c\u0438 \u043a\u043b\u0430\u0441\u0441\u0438\u0444\u0438\u043a\u0430\u0446\u0438\u0438/);assert.match(html,/Incoming stock query/);
 }finally{await rm(root,{recursive:true,force:true})}
});
