// Run the same pure contract used by the packaged final renderer, before saving.
import {readFile} from 'node:fs/promises';
import {validateRouteStructure} from '../examples/markdown-renderer/route-structure.mjs';
try {
 const value=JSON.parse(await readFile(process.argv[2],'utf8'));
 const result=validateRouteStructure(value.routes);
 console.log(JSON.stringify(result));
} catch(error) {
 console.log(JSON.stringify({status:'FAILED',code:error.code||'INVALID_ROUTE_INPUT',details:error.details||{message:error.message}}));
 process.exitCode=2;
}
