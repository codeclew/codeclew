(()=>{
 const ru=document.documentElement.lang==='ru',label=(en,ruText)=>ru?ruText:en;
 const main=document.querySelector('main.reader-guide');
 if(main&&!main.classList.contains('catalog-page')){
  const headings=[...main.querySelectorAll('h2')];
  if(headings.length>2){const layout=document.createElement('div');layout.className='guide-layout';main.before(layout);const nav=document.createElement('nav');nav.className='guide-toc';nav.setAttribute('aria-label',label('On this page','На этой странице'));const title=document.createElement('h2');title.textContent=label('On this page','На этой странице');nav.append(title);headings.forEach((h,i)=>{h.id=h.id||`guide-section-${i+1}`;const a=document.createElement('a');a.href=`#${h.id}`;a.textContent=h.textContent;nav.append(a);});layout.append(nav,main);}
 }
 const data=document.getElementById('catalog-data');
 if(data){const rows=JSON.parse(data.textContent),q=document.getElementById('catalog-query'),kind=document.getElementById('catalog-kind'),list=document.getElementById('catalog-results'),status=document.getElementById('catalog-status'),prev=document.getElementById('catalog-prev'),next=document.getElementById('catalog-next');let page=0;
  q.value=new URLSearchParams(location.search).get('q')||'';
  function render(){const query=q.value.trim().toLocaleLowerCase();const matched=rows.filter(r=>(!kind.value||r.kind===kind.value)&&`${r.title} ${r.id} ${r.kind}`.toLocaleLowerCase().includes(query));const pages=Math.max(1,Math.ceil(matched.length/20));page=Math.min(page,pages-1);list.replaceChildren();matched.slice(page*20,page*20+20).forEach(r=>{const li=document.createElement('li'),a=document.createElement('a'),meta=document.createElement('small');a.href=r.href;a.textContent=r.title;meta.textContent=`${ru?({Service:"Сервис",Process:"Процесс",Dataflow:"Движение данных"}[r.kind]||r.kind):r.kind} · ${r.id}`;li.append(a,meta);list.append(li);});status.textContent=matched.length?(ru?`Документов: ${matched.length} · страница ${page+1} из ${pages}`:`${matched.length} documents · page ${page+1} of ${pages}`):label('No matching documents. Try another name or type.','Документы не найдены. Попробуйте другое название или тип.');prev.disabled=page===0;next.disabled=page>=pages-1;}
  q.addEventListener('input',()=>{page=0;render();});kind.addEventListener('change',()=>{page=0;render();});prev.addEventListener('click',()=>{page--;render();});next.addEventListener('click',()=>{page++;render();});render();
 }
 document.querySelectorAll('.reader-links a').forEach(a=>{if(new URL(a.href).pathname===location.pathname)a.setAttribute('aria-current','page');});
})();
