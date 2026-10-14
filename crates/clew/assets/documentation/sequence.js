function wrap(text,max=28){const out=[''];for(const word of text.split(' ')){const i=out.length-1;if(out[i]&&out[i].length+word.length+1>max)out.push(word);else out[i]+=(out[i]?' ':'')+word;}return out;}
function sequenceWrap(text,max){
 const chunks=String(text??'').split(' ').flatMap(word=>{const points=Array.from(word),parts=[];for(let i=0;i<points.length;i+=max)parts.push(points.slice(i,i+max).join(''));return parts.length?parts:[''];});
 return wrap(chunks.join(' '),max);
}
// Project only visible sequence actions. The accepted event stream remains
// intact for readable behavior, source inspection and diagram exports.
function sequenceEvents(o){
 const root=[],stack=[{nodes:root}];
 for(const e of o.events){
  const current=stack[stack.length-1];
  if(['alt','loop','opt'].includes(e.kind)){
   const arm={marker:e,nodes:[]},frame={arms:[arm],end:null};
   current.nodes.push(frame);stack.push({frame,nodes:arm.nodes});
  }else if(e.kind==='else'&&current.frame?.arms[0].marker.kind==='alt'){
   const arm={marker:e,nodes:[]};current.frame.arms.push(arm);current.nodes=arm.nodes;
  }else if(e.kind==='end'&&stack.length>1){current.frame.end=e;stack.pop();}
  else if(e.kind!=='note'){current.nodes.push({event:e});}
 }
 const project=nodes=>nodes.flatMap(node=>{
  if(node.event)return [node.event];
  const arms=node.arms.map((arm,index)=>({...arm,index,events:project(arm.nodes)})).filter(arm=>arm.events.length);
  if(!arms.length)return [];
  const first=arms[0];
  // A surviving alternative stays conditional on the original guards. Keep
  // their complete text rather than making an else action unconditional or
  // inventing a logical negation of authored prose.
  const marker=first.index===0?first.marker:{...first.marker,kind:'alt',text:[t('Otherwise'),...node.arms.slice(0,first.index).map(arm=>arm.marker.text),first.marker.text.toLowerCase()==='otherwise'?'':first.marker.text].filter(Boolean).join(' · '),sourceIds:[...new Set(node.arms.slice(0,first.index+1).flatMap(arm=>arm.marker.sourceIds||[]))],dependencyIds:[...new Set(node.arms.slice(0,first.index+1).flatMap(arm=>arm.marker.dependencyIds||[]))]};
  return [marker,...first.events,...arms.slice(1).flatMap(arm=>[arm.marker,...arm.events]),...(node.end?[node.end]:[])];
 });
 return project(root);
}
let sequenceMeasureContext;
function sequenceLines(text,width,font,maxChars=Infinity,mono=false){
 if(sequenceMeasureContext===undefined)sequenceMeasureContext=document.createElement?.('canvas')?.getContext?.('2d')||null;
 const style=window.getComputedStyle?.(document.body),family=mono?(style?.getPropertyValue?.('--mono')?.trim()||'monospace'):(style?.fontFamily||'sans-serif');
 if(sequenceMeasureContext)sequenceMeasureContext.font=font+' '+family;
 const measure=value=>sequenceMeasureContext?sequenceMeasureContext.measureText(value).width:Array.from(value).length*(font.includes('8px')?4.9:6.8);
 const fits=value=>Array.from(value).length<=maxChars&&measure(value)<=Math.max(12,width);
 const lines=[];let line='';
 for(const word of String(text??'').split(' ')){
  if(fits(line+(line?' ':'')+word)){line+=(line?' ':'')+word;continue;}
  if(line){lines.push(line);line='';}
  if(fits(word)){line=word;continue;}
  for(const point of Array.from(word)){if(line&&!fits(line+point)){lines.push(line);line='';}line+=point;}
 }
 if(line||!lines.length)lines.push(line);
 return lines;
}
function sequenceLayout(o,availableWidth=800){
 const events=sequenceEvents(o),referenced=new Set(events.filter(e=>['message','return','declared'].includes(e.kind)).flatMap(e=>[e.from,e.to]));
 const participants=o.participants.filter(p=>referenced.has(p.id));
 const W=Math.max(160,Math.floor(availableWidth)),padding=16,gap=8,actorWidth=Math.min(152,Math.max(24,(W-padding*2-gap*Math.max(0,participants.length-1))/Math.max(1,participants.length)));
 const first=padding+actorWidth/2,last=W-padding-actorWidth/2,xs=Object.fromEntries(participants.map((p,i)=>[p.id,participants.length===1?W/2:first+i*(last-first)/Math.max(1,participants.length-1)]));
 const actors=participants.map(p=>({...p,width:actorWidth,titleLines:sequenceLines(p.label,actorWidth-16,'650 11px',20),technicalLines:sequenceLines(p.service||t('External participant'),actorWidth-16,'8px',28,true)}));
 const headerHeight=Math.max(48,...actors.map(p=>18+p.titleLines.length*14+p.technicalLines.length*11)),headerBottom=9+headerHeight;
 let y=headerBottom+37,stack=[],frames=[],rows=[];
 for(const e of events){
  const depth=stack.length,branch=['alt','else','loop','opt'].includes(e.kind),inset=Math.min(Math.max(0,depth-(e.kind==='else'?1:0))*12,Math.max(0,(W-160)/4)),left=24+inset;
  const a=xs[e.from]??W/2,b=xs[e.to]??W/2,self=a===b&&!branch&&e.kind!=='end',direction=W-padding-a>=a-padding?1:-1;
  const selfSpace=direction>0?W-padding-a:a-padding,selfWidth=Math.max(12,Math.min(180,selfSpace-8)),loopWidth=Math.max(8,Math.min(85,selfSpace-8));
  const labelWidth=self?selfWidth:Math.max(12,Math.abs(a-b)-16),mid=self?a+direction*selfWidth/2:(a+b)/2;
  const lines=e.kind==='end'?[]:sequenceLines(branch?`${t(e.kind)} · ${e.text}`:e.text,branch?W-left*2-12:labelWidth,branch?'650 9px':'650 11px');
  const arrowOffset=Math.max(39,(lines.length-1)*14+20),arrowBottom=arrowOffset+(a===b?17:0),captionOffset=arrowBottom+18;
  const captionLines=e.kind==='declared'?sequenceLines(t('Declared service interaction'),labelWidth,'650 9px'):[];
  const h=e.kind==='end'?18:branch?Math.max(40,lines.length*14+14):Math.max(72,(e.kind==='declared'?captionOffset+Math.max(0,captionLines.length-1)*14:arrowBottom)+28);
  rows.push({e,y,h,depth,left,lines,arrowOffset,captionOffset,captionLines,mid,labelWidth,direction,loopWidth});
  if(['alt','loop','opt'].includes(e.kind))stack.push({y,depth});
  if(e.kind==='end'){const frame=stack.pop();if(frame)frames.push({...frame,end:y+h-5});}
  y+=h;
 }
 return {W,H:y+25,xs,actors,headerHeight,headerBottom,frames:frames.sort((a,b)=>a.depth-b.depth),rows};
}
function diagram(o,availableWidth){
 const {W,H,xs,actors,headerHeight,headerBottom,frames,rows}=sequenceLayout(o,availableWidth);
 if(!actors.length||!rows.some(({e})=>['message','return','declared'].includes(e.kind)))return '';
 let svg=chromeHtml`<svg class="sequence-svg" data-sequence-operation="${esc(o.id)}" data-layout-width="${W}" style="width:100%;height:auto" viewBox="0 0 ${W} ${H}" role="group" aria-label="Sequence diagram: ${esc(o.title)}"><defs><marker id="arrow" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="6" markerHeight="6" orient="auto-start-reverse"><path d="M0 0 L8 4 L0 8Z" fill="#608365"/></marker></defs>`;
 svg+=frames.map(f=>{const inset=16+Math.min(f.depth*12,Math.max(0,(W-160)/4));return chromeHtml`<rect x="${inset}" y="${f.y-8}" width="${W-inset*2}" height="${f.end-f.y+8}" rx="4" fill="#fdfbf6" stroke="#e3d9c5"/>`;}).join('');
 svg+=actors.map(p=>chromeHtml`<line class="lifeline" x1="${xs[p.id]}" x2="${xs[p.id]}" y1="${headerBottom+4}" y2="${H-8}"/><rect x="${xs[p.id]-p.width/2}" y="9" width="${p.width}" height="${headerHeight}" rx="6" fill="#f1f6ed" stroke="#d6e1d0"/>${p.titleLines.map((line,i)=>chromeHtml`<text class="actor-label" text-anchor="middle" x="${xs[p.id]}" y="${29+i*14}">${esc(line)}</text>`).join('')}${p.technicalLines.map((line,i)=>chromeHtml`<text class="actor-technical" text-anchor="middle" x="${xs[p.id]}" y="${33+p.titleLines.length*14+i*11}">${esc(line)}</text>`).join('')}`).join('');
 let number=0;
 for(const {e,y,h,left,lines,arrowOffset,captionOffset,captionLines,mid,labelWidth,direction,loopWidth} of rows){if(e.kind==='end')continue;number++;let inside='';
 if(['alt','else','loop','opt'].includes(e.kind)){
  inside=chromeHtml`${e.kind==='else'?chromeHtml`<line x1="${left-12}" y1="${y-8}" x2="${W-left+12}" y2="${y-8}" stroke="#ddd2bd" stroke-dasharray="4 3"/>`:''}<rect class="hit-area" x="${left-5}" y="${y-5}" width="${W-left*2+10}" height="${h-9}" rx="3" fill="transparent"/>`+lines.map((line,i)=>chromeHtml`<text class="branch-title" x="${left}" y="${y+13+i*14}">${esc(line)}</text>`).join('');
 }else{
  const a=xs[e.from],b=xs[e.to],start=a===b?Math.min(a,a+direction*labelWidth):Math.min(a,b),width=a===b?labelWidth:Math.abs(a-b),arrowY=y+arrowOffset;
  inside=chromeHtml`<rect class="hit-area" x="${Math.max(0,start-12)}" y="${y-14}" width="${Math.min(W-Math.max(0,start-12),width+24)}" height="${h+5}" rx="4" fill="transparent"/><text class="step-number" x="${Math.max(4,Math.min(a,b)-12)}" y="${arrowY}">${number}</text>`+lines.map((line,i)=>chromeHtml`<text text-anchor="middle" x="${mid}" y="${y+i*14}">${esc(line)}</text>`).join('');
  const dash=['return','declared'].includes(e.kind)?'event-return':'';
  inside+=a===b?chromeHtml`<path class="event-arrow ${dash}" d="M${a} ${arrowY} h${direction*loopWidth} v17 h${-direction*loopWidth}" marker-end="url(#arrow)"/>`:chromeHtml`<line class="event-arrow ${dash}" x1="${a}" y1="${arrowY}" x2="${b}" y2="${arrowY}" marker-end="url(#arrow)"/>`;
  if(e.kind==='declared')inside+=captionLines.map((line,i)=>chromeHtml`<text class="branch-title" text-anchor="middle" x="${mid}" y="${y+captionOffset+i*14}">${esc(line)}</text>`).join('');
 }
 svg+=chromeHtml`<a href="#${esc(o.id)}" data-event="${esc(e.id)}" data-sequence-event="true" aria-label="${esc(e.text)}: inspect source"><title>${esc(e.text)}</title>${inside}</a>`;
 }return svg+chromeHtml`</svg>`;
}
