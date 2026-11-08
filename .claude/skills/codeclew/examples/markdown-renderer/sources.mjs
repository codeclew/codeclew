import {readFile} from 'node:fs/promises';
import {createHash} from 'node:crypto';
const sha=b=>'sha256:'+createHash('sha256').update(b).digest('hex');
const escape=s=>String(s).replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('"','&quot;');
const need=(ok,message)=>{if(!ok)throw Error('Invalid source footer: '+message)};

export async function loadSources(path){
 const value=JSON.parse(await readFile(path,'utf8'));
 need(value.schema==='codeclew-reader-sources/1','unsupported bundle');
 need(sha(await readFile(value.checkedIndex))===value.indexFileDigest,'checked index changed');
 const key=s=>JSON.stringify([s.service,s.file,s.revision,s.snapshot??null,s.startLine,s.endLine]);
 for(const p of value.pages){
  const raw=await readFile(p.map);need(sha(raw)===p.mapFileDigest,'checked map changed: '+p.page);
  const expected=new Map(JSON.parse(raw).claims.flatMap(c=>c.sources.map(s=>[key(s),s.textDigest])));
  const actual=new Map(p.groups.flatMap(g=>g.ranges.map(r=>[key({...g,...r}),sha(Buffer.from(r.text))])));
  need(actual.size===expected.size&&[...actual].every(([k,d])=>expected.get(k)===d),'excerpt bytes or coverage changed: '+p.page);
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
  const full=`${g.service}:${g.file} @ ${g.revision}${g.snapshot?' · '+g.snapshot:''}`;
  const links=g.ranges.map((r,j)=>{
   const id=`${prefix}-${i}-${j}`,label=r.startLine===r.endLine?String(r.startLine):`${r.startLine}–${r.endLine}`;
   details.push(`<details class="source-excerpt"><summary>${escape(full)} — ${label}</summary><pre id="${id}" tabindex="0" role="region" aria-label="${escape(g.file)} — ${label}"><code>${escape(r.text)}</code></pre></details>`);
   return `<a href="#${id}">${label}</a>`;
  });
  rows.push(`<li><span title="${escape(full)}">${escape(g.file.split('/').at(-1))}</span> — ${links.join('; ')}<small>${escape(g.service)} · ${escape(g.file)} · ${escape(String(g.revision).slice(0,12))}</small></li>`);
 });
 return `<section class="page-sources"><h2>${words.title}</h2>${rows.length?`<ul>${rows.join('')}</ul><details class="source-fragments"><summary>${escape(words.fragments)}</summary>${details.join('')}</details>`:`<p>${words.empty}</p>`}</section>`;
}

export const sourcesStyle='.page-sources{border-top:1px solid #ccd5df;margin-top:32px;padding-top:8px}.page-sources small{display:block;color:#435364;font-size:12px}.page-sources li{margin:12px 0}.source-excerpt{margin:8px 0}.source-excerpt summary{cursor:pointer;overflow-wrap:anywhere}.source-excerpt pre{white-space:pre;overflow:auto;max-height:28rem;padding:12px;background:#f1f4f7;font-size:13px}.source-excerpt code{font:inherit}';
export const sourcesScript=`const openSource=(hash=location.hash)=>{let id;try{id=decodeURIComponent(hash.slice(1))}catch{return}const detail=document.getElementById(id)?.closest('.source-excerpt');if(detail){for(let p=detail;p;p=p.parentElement)if(p.tagName==='DETAILS')p.open=true;detail.scrollIntoView()}};openSource();addEventListener('hashchange',()=>openSource());for(const a of document.querySelectorAll('.page-sources a'))a.addEventListener('click',()=>openSource(a.hash));`;
