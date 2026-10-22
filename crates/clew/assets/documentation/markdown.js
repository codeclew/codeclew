// Shared native/MDX presentation of authored prose. Evidence and source text never use this renderer.
(function(root,factory){
 const api=factory();
 if(typeof module==='object'&&module.exports)module.exports=api;
 else root.clewMarkdown=api;
})(typeof globalThis==='object'?globalThis:this,function(){
 'use strict';
 const escape=value=>String(value??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
 function url(value){
  const target=value.trim();
  if(!target||/[\s\u0000-\u001f\u007f\\]/.test(target)||target.startsWith('//'))return null;
  if(/^(https?:\/\/|mailto:)/i.test(target))return target;
  if(/^[a-z][a-z0-9+.-]*:/i.test(target)||target.includes(':'))return null;
  return target;
 }
 function inline(value,depth=0){
  const text=String(value??'');
  if(depth>6||text.length>65536)return escape(text);
  let out='',i=0;
  while(i<text.length){
   if(text[i]==='\\'&&i+1<text.length&&/[\\`*_[\]{}()|!]/.test(text[i+1])){out+=escape(text[i+1]);i+=2;continue;}
   if(text[i]==='`'){
    let count=1;while(text[i+count]==='`')count++;
    const marker='`'.repeat(count);let end=text.indexOf(marker,i+count);
    while(end!==-1&&(text[end-1]==='`'||text[end+count]==='`'))end=text.indexOf(marker,end+count);
    if(end!==-1){out+='<code>'+escape(text.slice(i+count,end).replace(/\n/g,' '))+'</code>';i=end+count;continue;}
    out+=escape(marker);i+=count;continue;
   }
   if(text[i]==='['&&(i===0||text[i-1]!=='!')){
    const close=text.indexOf('](',i+1),end=close===-1?-1:text.indexOf(')',close+2);
    if(end!==-1){
     const spec=text.slice(close+2,end).match(/^(\S+?)(?:\s+"([^"]*)")?$/),target=spec&&url(spec[1]);
     if(target){out+='<a href="'+escape(target)+'"'+(spec[2]?' title="'+escape(spec[2])+'"':'')+'>'+inline(text.slice(i+1,close),depth+1)+'</a>';i=end+1;continue;}
    }
   }
   const marker=text.startsWith('**',i)?'**':text[i]==='*'?'*':null;
   if(marker){const end=text.indexOf(marker,i+marker.length);if(end>i+marker.length){const tag=marker==='**'?'strong':'em';out+='<'+tag+'>'+inline(text.slice(i+marker.length,end),depth+1)+'</'+tag+'>';i=end+marker.length;continue;}}
   out+=escape(text[i++]);
  }
  return out;
 }
 function row(line){
  const cells=[];let current='',ticks=0,hasPipe=false;
  for(let i=0;i<line.length;i++){
   const c=line[i];
   if(c==='\\'&&i+1<line.length){current+=c+line[++i];continue;}
   if(c==='`'){let n=1;while(line[i+n]==='`')n++;if(!ticks)ticks=n;else if(ticks===n)ticks=0;current+='`'.repeat(n);i+=n-1;continue;}
   if(c==='|'&&!ticks){cells.push(current.trim());current='';hasPipe=true;}else current+=c;
  }
  cells.push(current.trim());
  if(line.trimStart().startsWith('|'))cells.shift();
  // An escaped final pipe belongs to the cell, not an optional border.
  if(/\|\s*$/.test(line)&&cells[cells.length-1]==='')cells.pop();
  return hasPipe?cells:null;
 }
 function separator(line){
  const cells=row(line);if(!cells||!cells.length||cells.some(c=>!/^:?-{3,}:?$/.test(c)))return null;
  return cells.map(c=>c.startsWith(':')&&c.endsWith(':')?'center':c.endsWith(':')?'right':c.startsWith(':')?'left':null);
 }
 function plain(value){
  return inline(value).replace(/<[^>]*>/g,'').replace(/&(amp|lt|gt|quot|#39);/g,(_,key)=>({amp:'&',lt:'<',gt:'>',quot:'"','#39':"'"}[key]));
 }
 function render(value,options={}){
  const text=String(value??'').replace(/\r\n?/g,'\n');
  if(text.length>65536)return '<p>'+escape(text)+'</p>';
  const lines=text.split('\n'),out=[];let i=0;
  while(i<lines.length){
   if(!lines[i].trim()){i++;continue;}
   if(/^(?: {4}|\t)/.test(lines[i])){
    const content=[];
    while(i<lines.length&&(!lines[i].trim()||/^(?: {4}|\t)/.test(lines[i])))content.push(lines[i++].replace(/^(?: {4}|\t)/,''));
    out.push('<pre><code>'+escape(content.join('\n'))+'</code></pre>');continue;
   }
   const fence=lines[i].match(/^\s{0,3}(`{3,}|~{3,})([^`]*)$/);
   if(fence){
    const content=[];i++;
    while(i<lines.length&&!new RegExp('^\\s{0,3}'+fence[1][0]+'{'+fence[1].length+',}\\s*$').test(lines[i]))content.push(lines[i++]);
    if(i<lines.length)i++;
    out.push('<pre><code>'+escape(content.join('\n'))+'</code></pre>');continue;
   }
   const header=row(lines[i]),align=i+1<lines.length?separator(lines[i+1]):null;
   if(header&&align&&header.length===align.length){
    let end=i+2;const body=[];let valid=true;
    while(end<lines.length&&lines[end].trim()){
     const cells=row(lines[end]);if(!cells)break;
     // Never discard extra accepted prose to make it fit the header.
     if(cells.length>header.length)valid=false;
     body.push(cells);end++;
    }
    if(valid){
     const attrs=n=>align[n]?' style="text-align:'+align[n]+'"':'';
     out.push('<div class="clew-md-table-wrap" tabindex="0" role="region" aria-label="'+escape(options.tableLabel||'Table')+'"><table class="clew-md-table'+(body.length?' has-rows':'')+'" role="table"><thead><tr>'+header.map((c,n)=>'<th scope="col"'+attrs(n)+'>'+inline(c)+'</th>').join('')+'</tr></thead><tbody>'+body.map(c=>'<tr>'+header.map((_,n)=>'<td data-label="'+escape(plain(header[n]))+'"'+attrs(n)+'>'+inline(c[n]||'')+'</td>').join('')+'</tr>').join('')+'</tbody></table></div>');
     i=end;continue;
    }
   }
   const paragraph=[lines[i++]];
   while(i<lines.length&&lines[i].trim()){
    if(/^\s{0,3}(`{3,}|~{3,})/.test(lines[i]))break;
    if(row(lines[i])&&i+1<lines.length&&separator(lines[i+1]))break;
    paragraph.push(lines[i++]);
   }
   out.push('<p>'+inline(paragraph.join('\n'))+'</p>');
  }
  return out.join('\n');
 }
 const installed=new WeakSet();
 function enhance(document,options={}){
  for(const node of document.querySelectorAll('[data-clew-markdown]:not([data-clew-markdown-ready])')){
   node.innerHTML=render(node.textContent,options);node.setAttribute('data-clew-markdown-ready','');
  }
  if(installed.has(document))return;installed.add(document);
  const open=(hash,moveFocus=false)=>{
   let id;try{id=decodeURIComponent(hash.replace(/^#/,''));}catch{return;}
   const target=document.getElementById(id);if(!target)return;
   if(target.tagName==='DETAILS')target.open=true;
   if(target.nextElementSibling?.tagName==='DETAILS')target.nextElementSibling.open=true;
   for(let parent=target.parentElement;parent;parent=parent.parentElement)if(parent.tagName==='DETAILS')parent.open=true;
   if(moveFocus){const detail=target.tagName==='DETAILS'?target:target.nextElementSibling?.tagName==='DETAILS'?target.nextElementSibling:null;detail?.querySelector?.('summary')?.focus?.({preventScroll:true});}
  };
  document.addEventListener('click',event=>{const link=event.target.closest?.('a[data-clew-field-details]');if(link)open(link.hash,true);});
  document.defaultView?.addEventListener('hashchange',()=>open(document.defaultView.location.hash));
  open(document.defaultView?.location.hash||'');
 }
 return {inline,render,enhance};
});
