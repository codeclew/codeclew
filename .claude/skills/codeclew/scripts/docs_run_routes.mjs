// Run the same pure contract used by the packaged final renderer, before saving.
import {readFile} from 'node:fs/promises';
import {validateRouteStructure,validateLocalRouteOrigin} from '../examples/markdown-renderer/route-structure.mjs';
try {
 const value=JSON.parse(await readFile(process.argv[2],'utf8'));
 const result=validateRouteStructure(value.routes);
 if(process.argv[3]){
  const context=JSON.parse(await readFile(process.argv[3],'utf8'));
  result.completions=value.routes.flatMap((r,i)=>{
   const completion=validateLocalRouteOrigin(r,i,context.docset,context.starts,context.completeMissingAnchor);
   return completion?[completion]:[];
  });
 }
 console.log(JSON.stringify(result));
} catch(error) {
 console.log(JSON.stringify({status:'FAILED',code:error.code||'INVALID_ROUTE_INPUT',details:error.details||{message:error.message}}));
 process.exitCode=2;
}
