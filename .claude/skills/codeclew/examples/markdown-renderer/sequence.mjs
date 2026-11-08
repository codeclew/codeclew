// A finite presentation of an authored sequence; no source or graph inference.
const escape=s=>s.replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('"','&quot;');
function need(ok,path,message){if(!ok)throw new Error(`Invalid sequence ${path}: ${message}`)}
function object(v,path,keys){need(v&&typeof v==='object'&&!Array.isArray(v),path,'expected object');need(Object.keys(v).every(k=>keys.includes(k)),path,'unknown property')}
function text(v,path){need(typeof v==='string'&&v.trim().length>0,path,'expected nonempty text');return v}
function target(v,path){
 text(v,path);need(/^[^/\\:?#\u0000-\u0020]+\.md(?:#[^\s]+)?$/u.test(v),path,'expected flat Markdown page with optional anchor');
 return v.replace(/\.md(?=#|$)/,'.html');
}

export function validateSequence(v){
 object(v,'/',['title','participants','steps']);text(v.title,'/title');
 need(Array.isArray(v.participants)&&v.participants.length>=2,'/participants','expected at least two participants');
 const ids=new Set();
 v.participants.forEach((p,i)=>{const path='/participants/'+i;object(p,path,['id','title','target']);text(p.id,path+'/id');need(!ids.has(p.id),path+'/id','duplicate participant');ids.add(p.id);text(p.title,path+'/title');if(p.target!==undefined)target(p.target,path+'/target')});
 need(Array.isArray(v.steps)&&v.steps.length>0,'/steps','expected nonempty steps');
 v.steps.forEach((s,i)=>{const path='/steps/'+i;object(s,path,['from','to','label','kind','target','condition']);need(ids.has(s.from),path+'/from','unknown participant');need(ids.has(s.to),path+'/to','unknown participant');text(s.label,path+'/label');need(['call','return','async','await','note'].includes(s.kind),path+'/kind','expected call, return, async, await or note');if(s.target!==undefined)target(s.target,path+'/target');if(s.condition!==undefined)text(s.condition,path+'/condition')});
 return v;
}

export function renderSequence(value,words={call:'Call',return:'Return',async:'Start without waiting',await:'Await completion',note:'Local step',steps:'Steps',participants:'Participants',diagram:'Diagram; scroll horizontally when needed'}){
 const v=validateSequence(value);
 const link=(href,label)=>href===undefined?escape(label):`<a href="${escape(target(href,'/target'))}">${escape(label)}</a>`;
 const actors=v.participants.map(p=>link(p.target,p.title)).join(' · ');
 const n=v.participants.length,index=id=>v.participants.findIndex(p=>p.id===id),title=id=>v.participants[index(id)].title;
 const lifelines=`<div class="sequence-lifelines" aria-hidden="true">${v.participants.map(()=>'<span></span>').join('')}</div>`;
 const rows=v.steps.map((s,i)=>{
  const from=index(s.from),to=index(s.to);let left=(Math.min(from,to)+.5)/n*100,width=Math.abs(from-to)/n*100;
  const self=from===to;if(self){left=from/n*100;width=100/n;}
  return `<div class="sequence-row"><div class="sequence-message ${s.kind}${from>to?' reverse':''}${self?' self':''}" role="group" aria-label="${escape(title(s.from))} → ${escape(title(s.to))}" style="margin-left:${left}%;width:${width}%"><span>${i+1}. ${link(s.target,s.label)}${(['async','await'].includes(s.kind)||(self&&s.kind==='note'))?`<small>${escape(words[s.kind])}</small>`:''}${s.condition?`<small>${escape(s.condition)}</small>`:''}</span>${self&&s.kind!=='note'?'<i class="sequence-loop" aria-hidden="true"></i>':''}</div></div>`;
 }).join('');
 const trace=v.steps.map((s,i)=>`<li><span class="sequence-direction">${escape(title(s.from))} → ${escape(title(s.to))} · ${escape(words[s.kind])}</span>${link(s.target,s.label)}${s.condition?`<p>${escape(s.condition)}</p>`:''}</li>`).join('');
 return `<figure class="clew-sequence"><figcaption>${escape(v.title)}</figcaption><p class="sequence-participants">${escape(words.participants)}: ${actors}</p><p class="sequence-hint">${escape(words.diagram)}</p><div class="sequence-scroll" tabindex="0" role="region" aria-label="${escape(words.diagram)}"><div class="sequence-diagram" role="group" aria-label="${escape(v.title)}" style="min-width:${Math.max(560,n*150)}px"><div class="sequence-actors">${v.participants.map(p=>`<span>${link(p.target,p.title)}</span>`).join('')}</div><div class="sequence-body">${lifelines}${rows}</div></div></div><details class="sequence-text" open><summary>${escape(words.steps)}</summary><ol>${trace}</ol></details></figure>\n`;
}

// Validate authored destinations against the actual rendered pages, including
// bound step headings. The existing full export checker checks them again.
export function checkSequenceTargets(sequences,pages){
 const byFile=new Map(pages.map(p=>[p.file,p.html]));
 for(const v of sequences)for(const item of [...v.participants,...v.steps]){
  if(item.target===undefined)continue;
  const split=item.target.indexOf('#'),file=split<0?item.target:item.target.slice(0,split);let anchor;
  try{anchor=decodeURIComponent(item.target.slice(split+1))}catch{need(false,item.target,'invalid anchor encoding')}
  need(byFile.has(file),item.target,'page does not exist');
  if(split<0)continue;
  const tags=byFile.get(file).match(/<[a-z][^>]*>/gi)||[];
  const ids=tags.flatMap(tag=>[...tag.matchAll(/\sid\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))/gi)].map(m=>m[1]??m[2]??m[3]));
  need(ids.filter(id=>id===anchor).length===1,item.target,'anchor missing or ambiguous');
 }
}
