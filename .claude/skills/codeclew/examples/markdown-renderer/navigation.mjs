// Explicit reader destinations, not inferred service ownership or a computation graph.
const escape=s=>s.replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('"','&quot;');
const kinds=new Set(['http','kafka','scheduled','background','manual','other']);
function need(ok,path,message){if(!ok)throw new Error(`Invalid navigation ${path}: ${message}`)}
function object(value,path,keys){need(value&&typeof value==='object'&&!Array.isArray(value),path,'expected object');need(Object.keys(value).every(k=>keys.includes(k)),path,'unknown property')}
function label(value,path){need(typeof value==='string'&&value.trim().length>0,path,'expected nonempty text');return value}
function array(value,path){need(Array.isArray(value),path,'expected array');return value}

export function validateNavigation(value,pages){
 object(value,'/',['schema','home','services','routes']);need(value.schema==='codeclew-markdown-navigation/1','/schema','unsupported schema');
 const byFile=new Map(pages.map(p=>[p.file,p]));
 function target(value,path){
  label(value,path);need(!/[\u0000-\u001f\\?]/u.test(value),path,'use a flat Markdown page and optional anchor');
  const split=value.indexOf('#'),file=split<0?value:value.slice(0,split),raw=split<0?'':value.slice(split+1);let fragment;
  try{fragment=decodeURIComponent(raw)}catch{need(false,path,'invalid anchor encoding')}
  need(!file.includes('/')&&!file.includes(':')&&file.endsWith('.md'),path,'use a flat Markdown page and optional anchor');
  need(byFile.has(file),path,'page does not exist');
  if(split>=0){
   need(fragment.length>0,path,'empty anchor');const tags=byFile.get(file).html.match(/<[a-z][^>]*>/gi)||[];
   const ids=tags.flatMap(tag=>[...tag.matchAll(/\sid\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))/gi)].map(m=>m[1]??m[2]??m[3]));
   need(ids.filter(id=>id===fragment).length===1,path,'anchor missing or ambiguous');
  }
  return {file,fragment,href:encodeURIComponent(file.replace(/\.md$/,'.html'))+(fragment?'#'+encodeURIComponent(fragment):'')};
 }
 function link(v,path,trigger=false){
  object(v,path,trigger?['title','target','kind','summary']:['title','target']);label(v.title,path+'/title');
  if(trigger){need(kinds.has(v.kind),path+'/kind','expected http, kafka, scheduled, background, manual or other');label(v.summary,path+'/summary')}
  return {...v,to:target(v.target,path+'/target')};
 }
 object(value.home,'/home',['title','target']);const home=link(value.home,'/home');
 const owners=new Map(),serviceIds=new Set();
 const services=array(value.services,'/services').map((s,i)=>{
  const path='/services/'+i;object(s,path,['id','title','overview','pages','starts','data']);label(s.id,path+'/id');label(s.title,path+'/title');
  need(!serviceIds.has(s.id),path+'/id','duplicate service');serviceIds.add(s.id);
  const files=array(s.pages,path+'/pages').map((f,j)=>{const t=target(f,path+'/pages/'+j);need(!t.fragment,path+'/pages/'+j,'page ownership cannot contain anchor');need(!owners.has(f),path+'/pages/'+j,'duplicate page ownership');owners.set(f,s.id);return f});
  const overview=target(s.overview,path+'/overview');need(!overview.fragment&&files.includes(overview.file),path+'/overview','overview must be an owned page');
  const starts=array(s.starts,path+'/starts').map((v,j)=>link(v,path+'/starts/'+j,true));
  const data=array(s.data,path+'/data').map((v,j)=>link(v,path+'/data/'+j));
  for(const v of [...starts,...data])need(files.includes(v.to.file),path,'local destination must be an owned page');
  return {...s,overview,starts,data};
 });
 const routeIds=new Set();
 const routes=array(value.routes,'/routes').map((r,i)=>{
  const path='/routes/'+i;object(r,path,['id','title','target','origin','participants']);label(r.id,path+'/id');label(r.title,path+'/title');need(!routeIds.has(r.id),path+'/id','duplicate route');routeIds.add(r.id);const to=target(r.target,path+'/target');need(!owners.has(to.file),path+'/target','shared route page cannot be owned by one service');
  function participant(v,path,origin=false){
   object(v,path,origin?['service','title','kind','target']:['service','title','role','target','section']);need(serviceIds.has(v.service),path+'/service','unknown service');label(v.title,path+'/title');const local=target(v.target,path+'/target');need(owners.get(local.file)===v.service,path+'/target','participant destination is not owned by this service');
   if(origin)need(kinds.has(v.kind),path+'/kind','invalid trigger kind');
   else label(v.role,path+'/role');
   const section=origin?null:target(v.section,path+'/section');if(section)need(section.file===to.file,path+'/section','participant section must belong to this route page');
   return {...v,to:local,section};
  }
  const origin=participant(r.origin,path+'/origin',true),participants=array(r.participants,path+'/participants').map((v,j)=>participant(v,path+'/participants/'+j));
  need(participants.some(p=>p.service===origin.service),path+'/origin/service','origin service must also be represented in participants');
  return {...r,to,origin,participants};
 });
 const used=new Set([home.to.file,...owners.keys(),...routes.map(r=>r.to.file)]);
 return {home,services,routes,owners,others:pages.filter(p=>!used.has(p.file)),byFile};
}

export function navigationView(nav,file,words){
 const service=nav.services.find(s=>s.id===nav.owners.get(file));
 function a(to,title,extra=''){
  const current=to.file===file&&!to.fragment?' aria-current="page"':'';
  return `<a href="${escape(to.href)}" data-page="${escape(to.file)}" data-fragment="${escape(to.fragment)}"${current}${extra}>${title}</a>`;
 }
 const badge=kind=>`<span class="trigger-kind">${escape(words['kind_'+kind])}</span>`;
 const group=(title,links)=>links.length?`<div class="navigation-category"><span class="navigation-label">${escape(title)}</span><ul>${links.map(x=>`<li>${x}</li>`).join('')}</ul></div>`:'';
 function participation(s){return nav.routes.flatMap(r=>r.participants.filter(p=>p.service===s.id).map(p=>({route:r,p})))}
 const menu=a(nav.home.to,escape(nav.home.title))+
  (nav.services.length?`<h2 class="navigation-label">${escape(words.services)}</h2>`:'')+
  nav.services.map(s=>{
   const linked=new Set([s.overview.file,...s.starts.map(v=>v.to.file),...s.data.map(v=>v.to.file)]);
   return `<details class="service-navigation"${s===service?' open':''}><summary>${escape(s.title)}</summary>${a(s.overview,escape(words.overview))}${group(words.starts,s.starts.map(v=>a(v.to,badge(v.kind)+escape(v.title))))}${group(words.data,s.data.map(v=>a(v.to,escape(v.title))))}${group(words.participation,participation(s).map(({route:r,p})=>a(p.section,escape(r.title)+'<span class="navigation-role">'+escape(p.title)+'</span>')))}${group(words.otherPages,s.pages.filter(f=>!linked.has(f)).map(f=>a({file:f,fragment:'',href:encodeURIComponent(f.replace(/\.md$/,'.html'))},escape(nav.byFile.get(f).title))))}</details>`;
  }).join('')+
  group(words.routes,nav.routes.map(r=>a(r.to,escape(r.title))))+
  group(words.otherPages,nav.others.map(p=>a({file:p.file,fragment:'',href:encodeURIComponent(p.file.replace(/\.md$/,'.html'))},escape(p.title))));
 const crumb=[a(nav.home.to,escape(nav.home.title))];
 if(service&&file!==service.overview.file)crumb.push(a(service.overview,escape(service.title)));
 else if(!service&&nav.routes.some(r=>r.to.file===file))crumb.push(`<span>${escape(words.routes)}</span>`);
 if(file!==nav.home.to.file)crumb.push(`<span aria-current="page">${escape(nav.byFile.get(file).title)}</span>`);
 const breadcrumb=`<nav class="reader-breadcrumb" aria-label="${escape(words.breadcrumb)}">${crumb.join('<span aria-hidden="true"> / </span>')}<span class="breadcrumb-section" hidden></span></nav>`;
 let hub='';
 if(service&&file===service.overview.file){
  const list=items=>`<ul class="hub-links">${items.map(x=>`<li>${x}</li>`).join('')}</ul>`;
  const roles=participation(service);
  const starts=service.starts.length?`<section><h2>${escape(words.starts)}</h2>${list(service.starts.map(v=>a(v.to,badge(v.kind)+escape(v.title))+`<p>${escape(v.summary)}</p>`))}</section>`:'';
  const data=service.data.length?`<h2>${escape(words.data)}</h2>${list(service.data.map(v=>a(v.to,escape(v.title))))}`:'';
  const shared=roles.length?`<h2>${escape(words.participation)}</h2>${list(roles.map(({route:r,p})=>a(p.section,escape(r.title)+' · '+escape(p.title))+`<p>${escape(p.role)}</p>`))}`:'';
  hub=starts||data||shared?`<section class="service-hub" aria-label="${escape(words.serviceNavigation)}"><div class="hub-columns">${starts}${data||shared?`<section>${data}${shared}</section>`:''}</div></section>`:'';
 }
 const shared=nav.routes.filter(r=>r.to.file===file);
 if(shared.length)hub=`<section class="service-hub" aria-label="${escape(words.routeParticipants)}"><div class="hub-columns">${shared.map(r=>`<section><h2>${a(r.to,escape(r.title))}</h2><p class="route-origin">${escape(words.origin)}: ${badge(r.origin.kind)}${a(r.origin.to,escape(nav.services.find(s=>s.id===r.origin.service).title)+' · '+escape(r.origin.title))}</p><ul class="hub-links">${r.participants.map(p=>`<li>${a(p.to,escape(nav.services.find(s=>s.id===p.service).title)+' · '+escape(p.title))}<p>${escape(p.role)} ${a(p.section,escape(words.inRoute))}</p></li>`).join('')}</ul></section>`).join('')}</div></section>`;
 return {menu,breadcrumb,hub};
}

export function insertHub(html,hub){
 if(!hub)return html;
 // Place discovery after the existing title/opening paragraph, without rewriting them.
 const opening=html.match(/^.*?<\/h1>(?:\s*<p>.*?<\/p>)?/s);
 const at=opening?opening[0].length:0;
 return html.slice(0,at)+hub+html.slice(at);
}
