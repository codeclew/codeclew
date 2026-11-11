import {readFile} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {dirname,resolve} from 'node:path';
const sha=b=>'sha256:'+createHash('sha256').update(b).digest('hex');
const escape=s=>String(s).replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('"','&quot;');
const need=(ok,message)=>{if(!ok)throw Error('Invalid source footer: '+message)};
const stable=v=>JSON.stringify(v,(_k,x)=>x&&typeof x==='object'&&!Array.isArray(x)?Object.fromEntries(Object.keys(x).sort().map(k=>[k,x[k]])):x);

export async function loadSources(path){
 const value=JSON.parse(await readFile(path,'utf8'));
 need(value.schema==='codeclew-reader-sources/1','unsupported bundle');
 need(sha(await readFile(resolve(dirname(path),value.checkedIndex)))===value.indexFileDigest,'checked index changed');
 const key=s=>JSON.stringify([s.service,s.file,s.revision,s.snapshot??null,s.startLine,s.endLine,
  ...(s.evidenceInput?[s.evidenceInput,s.bindingRepository??s.repository??null]:[])]);
 for(const p of value.pages){
  const raw=await readFile(resolve(dirname(path),p.map));need(sha(raw)===p.mapFileDigest,'checked map changed: '+p.page);
  const mapped=JSON.parse(raw);
  const expected=new Map(mapped.claims.flatMap(c=>(c.sources||[]).map(s=>[key(s),s.textDigest])));
  const actual=new Map(p.groups.flatMap(g=>g.ranges.map(r=>[key({...g,...r}),sha(Buffer.from(r.text))])));
  need(actual.size===expected.size&&[...actual].every(([k,d])=>expected.get(k)===d),'excerpt bytes or coverage changed: '+p.page);
  const declared=new Map(mapped.claims.flatMap(c=>(c.declaredEvidence||[]).map((h,i)=>[h,stable(c.declarations[i])])));
  const delivered=new Map((p.declarations||[]).map(d=>[d.cite,stable(d.record)]));
  need(delivered.size===declared.size&&[...delivered].every(([h,d])=>declared.get(h)===d),'declared bindings changed: '+p.page);
 }
 return value;
}

export function sourceFooter(bundle,file,markdown,html,ui){
 if(!bundle)return '';
 const page=bundle.pages.find(p=>p.page===file);
 need(page&&sha(Buffer.from(markdown))===page.markdownDigest,'Markdown changed: '+file);
 const words=ui||{title:'Sources',empty:'No bound sources for this page.',fragments:'Code fragments'};
 let prefix='clew-source';while(html.includes('id="'+prefix)||html.includes("id='"+prefix))prefix+='-excerpt';
 const details=[],rows=[];
 page.groups.forEach((g,i)=>{
  const owner=g.evidenceInput?`${g.evidenceInput} / ${g.service}`:g.service;
  const full=`${owner}:${g.file} @ ${g.revision}${g.snapshot?' · '+g.snapshot:''}`;
  const links=g.ranges.map((r,j)=>{
   const id=`${prefix}-${i}-${j}`,label=r.startLine===r.endLine?String(r.startLine):`${r.startLine}–${r.endLine}`;
   details.push(`<details class="source-excerpt"><summary>${escape(full)} — ${label}</summary><pre id="${id}" tabindex="0" role="region" aria-label="${escape(g.file)} — ${label}"><code>${escape(r.text)}</code></pre></details>`);
   return `<a href="#${id}">${label}</a>`;
  });
  rows.push(`<li><span title="${escape(full)}">${escape(g.file.split('/').at(-1))}</span> — ${links.join('; ')}<small>${escape(owner)} · ${escape(g.file)} · ${escape(String(g.revision).slice(0,12))}</small></li>`);
 });
 const declarations=[],seen=new Set();
 for(const d of page.declarations||[]){
  const r=d.record,k=JSON.stringify([r.evidenceInput,r.kind,r.localId,r.basis.recordDigest]);if(seen.has(k))continue;seen.add(k);
  const id=`${prefix}-declaration-${declarations.length}`;
  const owner=r.scope==='UNASSIGNED_ROOT'?r.evidenceInput:r.evidenceInput+' / '+r.service;
  const kind=r.kind==='TERM_CANDIDATE'?(words.concept||'Declared concept'):(words.catalog||'Declared catalog');
  declarations.push(`<li><details class="source-excerpt" id="${id}"><summary>${escape(r.title)}<small>${escape(owner)} · ${escape(kind)}</small></summary><p>${escape(r.id)}</p><small>${escape(words.snapshot||'Snapshot')}: ${escape(r.snapshot)}</small></details></li>`);
 }
 const code=rows.length?`<ul>${rows.join('')}</ul><details class="source-fragments"><summary>${escape(words.fragments)}</summary>${details.join('')}</details>`:declarations.length?'':`<p>${words.empty}</p>`;
 const declaredTitle=escape(words.declared||'Declared information');
 const declared=declarations.length?`${rows.length?`<h3>${declaredTitle}</h3>`:''}<ul>${declarations.join('')}</ul>`:'';
 return `<section class="page-sources"><h2>${declarations.length&&!rows.length?declaredTitle:words.title}</h2>${code}${declared}</section>`;
}

export const sourcesStyle='.page-sources{border-top:1px solid #ccd5df;margin-top:32px;padding-top:8px}.page-sources small{display:block;color:#435364;font-size:12px}.page-sources li{margin:12px 0}.source-excerpt{margin:8px 0}.source-excerpt summary{cursor:pointer;overflow-wrap:anywhere}.source-excerpt pre{white-space:pre;overflow:auto;max-height:28rem;padding:12px;background:#f1f4f7;font-size:13px}.source-excerpt code{font:inherit}';
export const sourcesScript=`const openSource=(hash=location.hash)=>{let id;try{id=decodeURIComponent(hash.slice(1))}catch{return}const detail=document.getElementById(id)?.closest('.source-excerpt');if(detail){for(let p=detail;p;p=p.parentElement)if(p.tagName==='DETAILS')p.open=true;detail.scrollIntoView()}};openSource();addEventListener('hashchange',()=>openSource());for(const a of document.querySelectorAll('.page-sources a'))a.addEventListener('click',()=>openSource(a.hash));`;
