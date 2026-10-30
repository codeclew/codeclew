import {readFile,writeFile,readdir,mkdir} from 'node:fs/promises';
import {resolve,join,basename} from 'node:path';
import {pathToFileURL} from 'node:url';
import {createRequire} from 'node:module';
const require=createRequire(import.meta.url);
const {marked}=await import(pathToFileURL(require.resolve('marked')));
const input=resolve(process.argv[2]);
const output=resolve(process.argv[3]);
const label=process.argv[4] || 'Documentation';
await mkdir(output,{recursive:true});
const files=(await readdir(input)).filter(x=>x.endsWith('.md')).sort();
const escape=s=>s.replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('"','&quot;');
for(const file of files){
 let html=await marked.parse(await readFile(join(input,file),'utf8'),{gfm:true});
 const seen=new Map();
 html=html.replace(/<h([1-6])>(.*?)<\/h\1>/gs,(_,level,title)=>{
  const base=title.replace(/<[^>]*>/g,'').toLowerCase().replace(/[^\p{L}\p{N}\s-]/gu,'').trim().replace(/\s+/g,'-');
  const n=seen.get(base)||0;seen.set(base,n+1);
  return `<h${level} id="${escape(base+(n?'-'+n:''))}">${title}</h${level}>`;
 }).replace(/href="([^"#]+)\.md(#[^"]*)?"/g,(_,path,anchor)=>`href="${path}.html${anchor||''}"`);
 const nav=files.map(f=>`<a href="${escape(f.replace(/\.md$/,'.html'))}">${escape(basename(f,'.md'))}</a>`).join(' · ');
 const page=`<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>${escape(label)}</title><style>body{margin:0;color:#16202b;background:#fff;font:17px/1.6 system-ui,sans-serif}header,main{max-width:92ch;margin:auto;padding:1.5rem}header{border-bottom:1px solid #dce2e8}nav{font-size:.9rem}h1,h2,h3{line-height:1.2;margin-top:1.7em}a{color:#175aab}code{font-family:ui-monospace,monospace;font-size:.88em}pre{padding:1rem;background:#f1f4f7;overflow:auto;border-radius:6px}table{border-collapse:collapse;display:block;overflow:auto}th,td{padding:.6rem;border:1px solid #ccd5df;text-align:left}blockquote{margin-left:0;padding-left:1rem;border-left:3px solid #ccd5df;color:#435364}</style><header><strong>${escape(label)}</strong><nav>${nav}</nav></header><main>${html}</main></html>`;
 await writeFile(join(output,file.replace(/\.md$/,'.html')),page);
}
console.log(JSON.stringify({input,output,files,renderer:'marked, same neutral CSS and ordinary Markdown for both samples'}));
