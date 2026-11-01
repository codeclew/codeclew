import {readFile,writeFile,readdir,mkdir} from 'node:fs/promises';
import {resolve,join,basename} from 'node:path';
import {pathToFileURL,fileURLToPath} from 'node:url';
import {createRequire} from 'node:module';
import {validateNavigation,navigationView,insertHub} from './navigation.mjs';
const require=createRequire(import.meta.url);
const {marked}=await import(pathToFileURL(require.resolve('marked')));
const escape=s=>s.replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('"','&quot;');

function renderMarkdown(markdown){
 let html=marked.parse(markdown,{gfm:true});
 const seen=new Map();
 html=html.replace(/<h([1-6])>(.*?)<\/h\1>/gs,(_,level,title)=>{
  const base=title.replace(/<[^>]*>/g,'').toLowerCase().replace(/[^\p{L}\p{N}\s-]/gu,'').trim().replace(/\s+/g,'-');
  const n=seen.get(base)||0;seen.set(base,n+1);
  return `<h${level} id="${escape(base+(n?'-'+n:''))}">${title}</h${level}>`;
 }).replace(/href="([^"#]+)\.md(#[^"]*)?"/g,(_,path,anchor)=>`href="${path}.html${anchor||''}"`);
 return html;
}

// Retained byte-for-byte for callers comparing existing neutral publications.
function neutralPage(html,files,label){
 const nav=files.map(f=>`<a href="${escape(f.replace(/\.md$/,'.html'))}">${escape(basename(f,'.md'))}</a>`).join(' · ');
 return `<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>${escape(label)}</title><style>body{margin:0;color:#16202b;background:#fff;font:17px/1.6 system-ui,sans-serif}header,main{max-width:92ch;margin:auto;padding:1.5rem}header{border-bottom:1px solid #dce2e8}nav{font-size:.9rem}h1,h2,h3{line-height:1.2;margin-top:1.7em}a{color:#175aab}code{font-family:ui-monospace,monospace;font-size:.88em}pre{padding:1rem;background:#f1f4f7;overflow:auto;border-radius:6px}table{border-collapse:collapse;display:block;overflow:auto}th,td{padding:.6rem;border:1px solid #ccd5df;text-align:left}blockquote{margin-left:0;padding-left:1rem;border-left:3px solid #ccd5df;color:#435364}</style><header><strong>${escape(label)}</strong><nav>${nav}</nav></header><main>${html}</main></html>`;
}

const plain=s=>s.replace(/<[^>]*>/g,'').replace(/&(?:amp|lt|gt|quot|#39);/g,x=>({'&amp;':'&','&lt;':'<','&gt;':'>','&quot;':'"','&#39;':"'"}[x]));
function decoration(svg){
 return svg.slice(svg.indexOf('<svg')).replace(/<title\b[^>]*>.*?<\/title>/gs,'').replace(/\s(?:role|aria-labelledby)="[^"]*"/g,'').replace('<svg','<svg aria-hidden="true" focusable="false"');
}

export async function renderDirectory(input,output,{label='Documentation',theme='codeclew',language='en',navigation}={}){
 if(!['codeclew','neutral'].includes(theme)||!['en','ru'].includes(language))throw new Error('Use --theme codeclew|neutral and --lang en|ru');
 input=resolve(input);output=resolve(output);
 const files=(await readdir(input)).filter(x=>x.endsWith('.md')).sort();
 const pages=await Promise.all(files.map(async file=>{
  const html=renderMarkdown(await readFile(join(input,file),'utf8'));
  const heading=html.match(/<h1\b[^>]*>(.*?)<\/h1>/s);
  return {file,html,title:heading?plain(heading[1]):basename(file,'.md')};
 }));
 const explicit=navigation===undefined?null:validateNavigation(typeof navigation==='string'?JSON.parse(await readFile(navigation,'utf8')):navigation,pages);
 await mkdir(output,{recursive:true});
 let assets;
 if(theme==='codeclew'){
  const directory=fileURLToPath(new URL('./assets/',import.meta.url));
  const [reader,type,style,icon,cat]=await Promise.all(['reader.css','typography.css','markdown.css','favicon.svg','cat-standing.svg'].map(x=>readFile(join(directory,x),'utf8')));
  assets={style:reader+'\n'+type+'\n'+style,icon,cat};
  await writeFile(join(output,'favicon.svg'),icon);
 }
 const words=language==='ru'?JSON.parse(await readFile(new URL('./assets/ru.json',import.meta.url),'utf8')):{skip:'Skip to content',pages:'Pages',navigation:'Documentation pages',table:'Table; scroll horizontally when needed',documentation:'Documentation',services:'Services',overview:'Overview',starts:'Starts here',data:'Data',participation:'Shared-route participation',routes:'Cross-service routes',otherPages:'Other pages',breadcrumb:'Current location',serviceNavigation:'Service navigation',routeParticipants:'Route participants',origin:'Route origin',inRoute:'In this route',kind_http:'HTTP',kind_kafka:'Kafka',kind_scheduled:'Scheduled',kind_background:'Background',kind_manual:'Manual',kind_other:'Other'};
 const ordered=[...pages].sort((a,b)=>Number(b.file==='index.md')-Number(a.file==='index.md'));
 const home=explicit?explicit.home.to.href:files.includes('index.md')?'index.html':files[0]?.replace(/\.md$/,'.html');
 for(const {file,html,title} of pages){
  let page;
  if(theme==='neutral')page=neutralPage(html,files,label);
  else{
   let mainId='clew-reader-main';while(html.includes(`id="${mainId}"`))mainId+='-content';
   const view=explicit?navigationView(explicit,file,words):null;
   const links=view?view.menu:ordered.map(p=>`<a href="${escape(p.file.replace(/\.md$/,'.html'))}"${p.file===file?' aria-current="page"':''}>${escape(p.title)}</a>`).join('');
   const script=`const menu=document.querySelector('.guide-local-navigation');const narrow=matchMedia('(max-width:950px)');const resize=()=>{menu.open=!narrow.matches};resize();narrow.addEventListener('change',resize);for(const table of document.querySelectorAll('main table')){const region=document.createElement('div');region.className='table-scroll';region.tabIndex=0;region.setAttribute('role','region');region.setAttribute('aria-label',${JSON.stringify(words.table)});table.before(region);region.append(table);for(const th of table.querySelectorAll('thead th'))th.scope='col'}`;
   const locationScript=`const syncLocation=()=>{let id='';try{id=decodeURIComponent(location.hash.slice(1))}catch{}const target=id?document.getElementById(id):null;const heading=target&&target.tagName!=='MAIN'?(target.matches('h1,h2,h3,h4,h5,h6')?target:target.querySelector('h1,h2,h3,h4,h5,h6')):null;const crumb=document.querySelector('.breadcrumb-section');if(crumb){crumb.hidden=!heading;crumb.replaceChildren();if(heading){crumb.append(' / ');const link=document.createElement('a');link.href='#'+encodeURIComponent(id);link.textContent=heading.textContent;link.setAttribute('aria-current','location');crumb.append(link)}}for(const link of document.querySelectorAll('.guide-local-navigation a[data-page]')){if(link.dataset.page===document.body.dataset.readerPage&&link.dataset.fragment){if(link.dataset.fragment===id)link.setAttribute('aria-current','location');else link.removeAttribute('aria-current')}}};syncLocation();addEventListener('hashchange',syncLocation);`;
   page=`<!doctype html><html lang="${language}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="theme-color" content="#12334e"><title>${escape(title)} — ${escape(label)}</title><link rel="icon" type="image/svg+xml" href="favicon.svg"><style>${assets.style}</style></head><body class="markdown-reader" data-reader-page="${escape(file)}"><a class="skip-link" href="#${mainId}">${escape(words.skip)}</a><header class="reader-nav"><a class="reader-brand" href="${escape(home||'#')}">${decoration(assets.icon)}<span>Codeclew</span></a><span class="reader-project">${escape(label)}</span></header><div class="guide-layout"><aside class="guide-toc"><details class="guide-local-navigation" open><summary>${escape(words.pages)}</summary><nav aria-label="${escape(words.navigation)}">${links}</nav></details></aside><main class="reader-guide" id="${mainId}">${view?view.breadcrumb:''}${insertHub(html,view?.hub)}<footer class="markdown-footer">${decoration(assets.cat)}<span>Codeclew · ${escape(words.documentation)}</span></footer></main></div><script>${script}${view?locationScript:''}</script></body></html>`;
  }
  await writeFile(join(output,file.replace(/\.md$/,'.html')),page);
 }
 return {input,output,files,theme,language,renderer:'marked; authored Markdown and checked source components retained'};
}

if(process.argv[1]&&pathToFileURL(resolve(process.argv[1])).href===import.meta.url){
 const args=process.argv.slice(2);const input=args.shift(),output=args.shift();
 if(!input||!output)throw new Error('Usage: node render.mjs INPUT OUTPUT [LABEL] [--theme codeclew|neutral] [--lang en|ru] [--navigation FILE]');
 const options={};if(args.length&&!args[0].startsWith('--'))options.label=args.shift();
 while(args.length){const key=args.shift(),value=args.shift();if(!value)throw new Error('Missing renderer option value: '+key);if(key==='--theme')options.theme=value;else if(key==='--lang')options.language=value;else if(key==='--navigation')options.navigation=value;else throw new Error('Unknown renderer option: '+key)}
 console.log(JSON.stringify(await renderDirectory(input,output,options)));
}
