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
const normalizePredicate=text=>text.replace(/\s+/gu,'');
const normalizeLines=text=>text.replace(/\r\n?/g,'\n');
const decimal=/^(?:0|-?[1-9][0-9]{0,9})$/;

function paragraphRanges(markdown,tokens) {
 const paragraphs=[];
 let offset=0;
 for(const token of tokens) {
  if(!markdown.startsWith(token.raw,offset))break;
  const end=offset+token.raw.length;
  if(token.type==='paragraph')paragraphs.push({token,start:offset,end});
  offset=end;
 }
 return paragraphs;
}

function checkLiteralReturn(row,page) {
 const predicate=normalizePredicate(row.predicate);
 const start=normalizeLines(page.raw.subarray(0,row.startByte).toString()).length;
 const end=normalizeLines(page.raw.subarray(0,row.endByte).toString()).length;
 const answers=[];
 for(const paragraph of page.paragraphs) {
  const {token}=paragraph;
  if(paragraph.start<start||paragraph.end>end)continue;
  const spans=[];
  marked.walkTokens(token.tokens,value=>{if(value.type==='codespan')spans.push(value.text)});
  for(const [index,span] of spans.entries()) {
   if(normalizePredicate(span)===predicate)answers.push(spans[index+1]);
  }
 }
 const details={check:row.id,page:row.page,claim:row.claim,predicate:row.predicate};
 need(answers.length===1&&decimal.test(answers[0]??''),'LITERAL_RETURN_VALUE_NOT_ADDRESSABLE',
  {...details,nextAction:'In the checked claim, write the condition and then the result as consecutive inline code spans in one paragraph.'});
 need(answers[0]===row.literal,'LITERAL_RETURN_VALUE_MISMATCH',
  {...details,expected:row.literal,documented:answers[0]});
 return {page:row.page,claim:row.claim};
}

try {
 if(input==='--probe') {
  console.log(JSON.stringify({node:process.version,markedEntryDigest:'sha256:'+createHash('sha256').update(await readFile(entry)).digest('hex')}));
 } else {
  const value=JSON.parse(await readFile(input,'utf8'));
  const sequences={};
  const returnPages=new Map((value.literalReturns||[]).map(row=>[row.page,null]));
  for(const [name,markdown] of Object.entries(value.pages)) {
   sequences[name]=[];
   const normalized=normalizeLines(markdown);
   const tokens=marked.lexer(normalized,{gfm:true});
   if(returnPages.has(name))returnPages.set(name,{raw:Buffer.from(markdown),paragraphs:paragraphRanges(normalized,tokens)});
   marked.walkTokens(tokens,token=>{
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
  const checkedReturns=Object.fromEntries((value.literalReturns||[]).map(row=>[row.id,checkLiteralReturn(row,returnPages.get(row.page))]));
  console.log(JSON.stringify({status:'ENTRY_SEQUENCES_CHECKED',entries:counts,literalReturns:checkedReturns,semanticTruthAssessed:false}));
 }
} catch(error) {
 console.log(JSON.stringify({status:'FAILED',code:error.code||'INVALID_SEQUENCE_INPUT',details:error.details||{message:error.message}}));
 process.exitCode=2;
}
