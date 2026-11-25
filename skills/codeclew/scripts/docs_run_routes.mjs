// Check authored route declarations without rendering pages or resolving retained source.
import {readFileSync} from 'node:fs';
import {resolve} from 'node:path';
import {pathToFileURL} from 'node:url';

try {
 const renderer=resolve(process.argv[2],'docset-routes.mjs');
 const {validateDraftRouteCatalog}=await import(pathToFileURL(renderer));
 const {registry,navigation,files,services}=JSON.parse(readFileSync(0,'utf8'));
 const docset=registry?.catalog?.docset;
 const matching=Array.isArray(registry?.docsets)?registry.docsets.filter(d=>d?.id===docset):[];
 const local=matching[0];
 const members=new Set(Array.isArray(local?.services)?local.services.map(s=>s?.id):[]);
 if(typeof docset!=='string'||matching.length!==1||members.size!==services.length||!services.every(s=>members.has(s))){
  throw Object.assign(Error('Declare the local catalog docset with the exact service inventory from the job.'),
   {code:'LOCAL_PROCESS_CATALOG_REQUIRED',path:'/catalog/docset',details:{docset,expectedServices:services}});
 }
 const model=validateDraftRouteCatalog(registry,navigation,files,docset,local.version);
 const localNarratives=model.routes.some(r=>r.to.kind==='LOCAL_DOCSET');
 console.log(JSON.stringify({status:'ROUTE_DECLARATIONS_CHECKED',docset,version:local.version,localNarratives}));
} catch(error) {
 console.log(JSON.stringify({status:'FAILED',code:error.code||'INVALID_ROUTE_DECLARATIONS',
  details:{file:error.file||'routes.json',path:error.path,message:error.message,...error.details}}));
 process.exitCode=2;
}
