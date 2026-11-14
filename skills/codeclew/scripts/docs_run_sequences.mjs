// Inspect actual Markdown tokens with the packaged renderer's parser/validator.
// Entry identity is a structural binding, never proof of the diagram's meaning.
import {readFile} from 'node:fs/promises';
import {createRequire} from 'node:module';
import {resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
import {createHash} from 'node:crypto';

const [renderer,input]=process.argv.slice(2);
const require=createRequire(resolve(renderer,'render.mjs'));
const entry=require.resolve('marked');
const {marked}=await import(pathToFileURL(entry));
const {validateSequence}=await import(pathToFileURL(resolve(renderer,'sequence.mjs')));
const need=(condition,code,details={})=>{if(!condition)throw Object.assign(Error(code),{code,details})};
try {
 if(input==='--probe') {
  console.log(JSON.stringify({node:process.version,markedEntryDigest:'sha256:'+createHash('sha256').update(await readFile(entry)).digest('hex')}));
 } else {
  const value=JSON.parse(await readFile(input,'utf8'));
  const sequences={};
  for(const [name,markdown] of Object.entries(value.pages)) {
   sequences[name]=[];
   marked.walkTokens(marked.lexer(markdown,{gfm:true}),token=>{
    if(token.type==='code'&&token.lang?.trim()==='clew-sequence') {
     let sequence;
     try{sequence=validateSequence(JSON.parse(token.text))}catch(error){need(false,'INVALID_REQUIRED_SEQUENCE',{page:name,message:error.message})}
     sequences[name].push(sequence);
    }
   });
  }
  const counts={};
  for(const requirement of value.requirements) {
   if(requirement.kind!=='ENTRY')continue;
   const account=value.accounts.find(row=>row.requirement===requirement.id);
   const selected=sequences[requirement.page]?.[account?.diagram];
   need(selected,'REQUIRED_SEQUENCE_MISSING',{requirement:requirement.id,page:requirement.page});
   const handler=selected.participants.find(p=>p.id===requirement.handlerId);
   const first=selected.steps[0],target=requirement.page+'#handler';
   need(handler?.target===target&&first.to===requirement.handlerId&&first.target===target,
    'SEQUENCE_ENTRY_MISMATCH',{requirement:requirement.id,page:requirement.page});
   if(requirement.method)need(first.kind==='call'&&first.from!==first.to&&first.label===requirement.method+' '+requirement.path,
    'SEQUENCE_HTTP_ENTRY_MISMATCH',{requirement:requirement.id,page:requirement.page});
   counts[requirement.id]={page:requirement.page,diagram:account.diagram,steps:selected.steps.length};
  }
  console.log(JSON.stringify({status:'ENTRY_SEQUENCES_CHECKED',entries:counts,semanticTruthAssessed:false}));
 }
} catch(error) {
 console.log(JSON.stringify({status:'FAILED',code:error.code||'INVALID_SEQUENCE_INPUT',details:error.details||{message:error.message}}));
 process.exitCode=2;
}
