// Use the admitted final renderer, not a parallel Markdown/diagram implementation.
import {readFile} from 'node:fs/promises';
import {pathToFileURL} from 'node:url';
import {resolve} from 'node:path';
const [renderer,request]=process.argv.slice(2);
try {
 const v=JSON.parse(await readFile(request,'utf8'));
 const {renderDirectory}=await import(pathToFileURL(resolve(renderer,'render.mjs')));
 const result=await renderDirectory(v.input,v.output,v.options);
 console.log(JSON.stringify({status:'LOCAL_RENDER_CHECKED',result}));
} catch(error) {
 console.log(JSON.stringify({status:'FAILED',code:error.code||'LOCAL_RENDER_FAILED',details:error.details||{message:error.message}}));
 process.exitCode=2;
}
