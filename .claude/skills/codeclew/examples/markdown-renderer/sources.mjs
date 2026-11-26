import {readFile} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {dirname,resolve} from 'node:path';
const sha=b=>'sha256:'+createHash('sha256').update(b).digest('hex');
const escape=s=>String(s).replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('"','&quot;');
const need=(ok,message)=>{if(!ok)throw Error('Invalid source footer: '+message)};
const stable=v=>JSON.stringify(v,(_k,x)=>x&&typeof x==='object'&&!Array.isArray(x)?Object.fromEntries(Object.keys(x).sort().map(k=>[k,x[k]])):x);
const sourceKey=(s,withOccurrence=true)=>JSON.stringify([s.service,s.file,s.revision,s.snapshot??null,s.startLine,s.endLine,
 ...(s.evidenceInput?[s.evidenceInput,s.bindingRepository??s.repository??null]:[]),
 ...(withOccurrence&&s.occurrence?[[s.occurrence.snapshot,s.occurrence.blob,s.occurrence.startByte,s.occurrence.endByte]]:[])]);
const checkedClaims=new WeakMap();

export async function loadSources(path){
 const value=JSON.parse(await readFile(path,'utf8'));
 need(value.schema==='codeclew-reader-sources/1','unsupported bundle');
 need(sha(await readFile(resolve(dirname(path),value.checkedIndex)))===value.indexFileDigest,'checked index changed');
 const claims=new Map();
 for(const p of value.pages){
  const raw=await readFile(resolve(dirname(path),p.map));need(sha(raw)===p.mapFileDigest,'checked map changed: '+p.page);
  const mapped=JSON.parse(raw);
  const excerpts=p.groups.flatMap(g=>g.ranges.map(r=>({...g,...r})));
  // Older bundles carry only line coordinates and must still agree with every bound digest.
  const exactLines=new Set(excerpts.filter(s=>s.occurrence).map(s=>sourceKey(s,false)));
  const boundKey=s=>sourceKey(s,exactLines.has(sourceKey(s,false)));
  const expected=new Map();
  for(const s of mapped.claims.flatMap(c=>c.sources||[])){
   const key=boundKey(s);
   need(!expected.has(key)||expected.get(key)===s.textDigest,'conflicting bound excerpts: '+p.page);
   expected.set(key,s.textDigest);
  }
  const actual=new Map(excerpts.map(s=>[sourceKey(s),sha(Buffer.from(s.text))]));
  need(actual.size===excerpts.length,'duplicate excerpt: '+p.page);
  need(actual.size===expected.size&&[...actual].every(([k,d])=>expected.get(k)===d),'excerpt bytes or coverage changed: '+p.page);
  const declared=new Map(mapped.claims.flatMap(c=>(c.declaredEvidence||[]).map((h,i)=>[h,stable(c.declarations[i])])));
  const delivered=new Map((p.declarations||[]).map(d=>[d.cite,stable(d.record)]));
  need(delivered.size===declared.size&&[...delivered].every(([h,d])=>declared.get(h)===d),'declared bindings changed: '+p.page);
  claims.set(p.page,new Map(mapped.claims.filter(c=>typeof c.id==='string'&&/^source-outcome-[1-9][0-9]*$/.test(c.id)&&c.sources?.length)
   .map(c=>[c.id,c.sources.map(boundKey)])));
 }
 checkedClaims.set(value,claims);
 return value;
}

export function sourceFooter(bundle,file,markdown,html,ui,anchors){
 if(!bundle)return '';
 const page=bundle.pages.find(p=>p.page===file);
 need(page&&sha(Buffer.from(markdown))===page.markdownDigest,'Markdown changed: '+file);
 const words=ui||{title:'Sources',empty:'No bound sources for this page.',fragments:'Code fragments'};
 let prefix='clew-source';while(html.includes('id="'+prefix)||html.includes("id='"+prefix))prefix+='-excerpt';
 const details=[],rows=[];
 page.groups.forEach((g,i)=>{
  const owner=g.evidenceInput?`${g.evidenceInput} / ${g.service}`:g.service;
  const full=`${owner}:${g.file} @ ${g.revision}${g.snapshot?' · '+g.snapshot:''}`;
  const count=new Map();
  for(const r of g.ranges){
   const lines=`${r.startLine}:${r.endLine}`;
   count.set(lines,(count.get(lines)||0)+1);
  }
  const links=g.ranges.map((r,j)=>{
   const id=`${prefix}-${i}-${j}`,lines=r.startLine===r.endLine?String(r.startLine):`${r.startLine}–${r.endLine}`;
   const label=r.occurrence&&count.get(`${r.startLine}:${r.endLine}`)>1
    ? escape(`${lines} · ${words.bytes||'bytes'} [${r.occurrence.startByte}, ${r.occurrence.endByte})`) : lines;
   anchors?.set(sourceKey({...g,...r}),
    `<a href="#${id}" title="${escape(full)}">${escape(g.service)} / ${escape(g.file.split('/').at(-1))}:${label}</a>`);
   details.push(`<details class="source-excerpt"><summary>${escape(full)} — ${label}</summary><pre id="${id}" tabindex="0" role="region" aria-label="${escape(g.file)} — ${label}"><code>${escape(r.text)}</code></pre></details>`);
   return `<a href="#${id}">${label}</a>`;
  });
  rows.push(`<li><span title="${escape(full)}">${escape(g.file.split('/').at(-1))}</span> — ${links.join('; ')}<small>${escape(owner)} · ${escape(g.file)} · ${escape(String(g.revision).slice(0,12))}</small></li>`);
 });
 const declarations=[],seen=new Set();
 for(const d of page.declarations||[]){
  const r=d.record,k=JSON.stringify([r.evidenceInput,r.kind,r.localId,r.basis.recordDigest,r.reviewRun??null,r.basis.answerDigest??null]);if(seen.has(k))continue;seen.add(k);
  const id=`${prefix}-declaration-${declarations.length}`;
  const owner=r.scope==='UNASSIGNED_ROOT'?r.evidenceInput:r.evidenceInput+' / '+r.service;
  const kind=r.basisKind==='SAVED_GLOSSARY'?(words.glossary||'Saved glossary'):r.kind==='TERM_CANDIDATE'?(words.concept||'Declared concept'):(words.catalog||'Declared catalog');
  const qualification=r.basisKind==='SAVED_GLOSSARY'&&r.originalEntry.definition.uncertainty?`<p>${escape(r.originalEntry.definition.uncertainty)}</p>`:'';
  declarations.push(`<li><details class="source-excerpt" id="${id}"><summary>${escape(r.title)}<small>${escape(owner)} · ${escape(kind)}</small></summary>${qualification}<p>${escape(r.id)}</p><small>${escape(words.snapshot||'Snapshot')}: ${escape(r.snapshot)}</small></details></li>`);
 }
 const code=rows.length?`<ul>${rows.join('')}</ul><details class="source-fragments"><summary>${escape(words.fragments)}</summary>${details.join('')}</details>`:declarations.length?'':`<p>${words.empty}</p>`;
 const glossaryOnly=(page.declarations||[]).length&&(page.declarations||[]).every(d=>d.record.basisKind==='SAVED_GLOSSARY');
 const declaredTitle=escape(glossaryOnly?(words.glossary||'Saved glossary'):(words.declared||'Declared information'));
 const declared=declarations.length?`${rows.length?`<h3>${declaredTitle}</h3>`:''}<ul>${declarations.join('')}</ul>`:'';
 return `<section class="page-sources"><h2>${declarations.length&&!rows.length?declaredTitle:words.title}</h2>${code}${declared}</section>`;
}

export function withSourceFooter(bundle,file,markdown,html,ui){
 const claims=bundle&&checkedClaims.get(bundle)?.get(file);
 const anchors=claims?.size&&new Map();
 const footer=sourceFooter(bundle,file,markdown,html,ui,anchors||null);
 if(!anchors)return html+footer;
 return html.replace(/<span data-clew-outcome-source="(source-outcome-[1-9][0-9]*)"><\/span>/g,(marker,id)=>{
  if(!claims.has(id))return marker;
  const links=[...new Set(claims.get(id))].map(key=>anchors.get(key));
  need(links.every(Boolean),'outcome source link unavailable: '+id);
  const title=escape(ui?.title||'Sources'), list=links.join(', ');
  return links.length>4
   ? `<details class="operation-outcome-sources"><summary>${title} (${links.length})</summary><p>${list}</p></details>`
   : `<small class="operation-outcome-sources">${title} · ${list}</small>`;
 })+footer;
}

export const sourcesStyle='.page-sources{border-top:1px solid #ccd5df;margin-top:32px;padding-top:8px}.page-sources small{display:block;color:#435364;font-size:12px}.page-sources li{margin:12px 0}.source-excerpt{margin:8px 0}.source-excerpt summary{cursor:pointer;overflow-wrap:anywhere}.source-excerpt pre{white-space:pre;overflow:auto;max-height:28rem;padding:12px;background:#f1f4f7;font-size:13px}.source-excerpt code{font:inherit}.operation-outcomes{margin-block:1rem}.operation-outcomes summary{cursor:pointer}.operation-outcomes li{margin-block:.9rem}.operation-outcomes p{margin:.25rem 0}.operation-outcome-sources{display:block;overflow-wrap:anywhere}';
export const sourcesScript=`const openSource=(hash=location.hash)=>{let id;try{id=decodeURIComponent(hash.slice(1))}catch{return}const detail=document.getElementById(id)?.closest('.source-excerpt');if(detail){for(let p=detail;p;p=p.parentElement)if(p.tagName==='DETAILS')p.open=true;detail.scrollIntoView()}};openSource();addEventListener('hashchange',()=>openSource());for(const a of document.querySelectorAll('.page-sources a,.operation-outcome-sources a'))a.addEventListener('click',()=>openSource(a.hash));`;
