// Pure declaration checks. Shared by route recording and the final renderer.
// No source acquisition, semantic inference, target fabrication or mutation.
export const triggerKinds=['http','kafka','scheduled','background','manual','other'];
export const outcomeKinds=['normal','error','handoff','unknown'];
// Both recording and rendering require the same exact registered start address.
// Only recording may complete an omitted anchor from one unambiguous start.
export function validateLocalRouteOrigin(route,index,docset,starts,completeMissingAnchor=false){
 if(route.origin?.status!=='known')return;
 const oi=route.occurrences.findIndex(o=>o.id===route.origin.occurrence),o=route.occurrences[oi];
 if(o?.actor?.kind!=='service'||o.actor.docset!==docset)return;
 const target=o.target,expected=starts.filter(s=>s.service===o.actor.service);
 const matches=expected.filter(s=>s.kind===route.origin.kind&&s.target.docset===target?.docset&&s.target.page===target?.page);
 if(matches.some(s=>(s.target.anchor??'')===(target?.anchor??'')))return;
 if(completeMissingAnchor&&target&&!Object.hasOwn(target,'anchor')&&matches.length===1)
  return {routeIndex:index,occurrenceIndex:oi,anchor:matches[0].target.anchor};
 const issue={code:'ROUTE_ORIGIN_START_MISMATCH',routeId:route.id,occurrenceId:o.id,
  field:`/routes/${index}/occurrences/${oi}/target`,actual:{kind:route.origin.kind,target},expected,
  message:'local origin is not a documented start',
  recovery:'Correct the named origin occurrence target/kind with ./docs route --input FILE using an exact registered start from ./docs show --entries. Preserve other routes and source-backed prose.'};
 throw Object.assign(Error('Invalid docset routes '+issue.field+': '+issue.message),
  {code:'ROUTE_DECLARATION_INVALID',details:{issues:[issue],recovery:issue.recovery}});
}
export function validateRouteStructure(routes){
 const issues=[];
 const problem=(ok,code,field,expected,actual,route,outcome,message)=>{
  if(!ok)issues.push({code,field,expected,actual:actual??null,
   ...(route?.id?{routeId:route.id}:{}),...(outcome?.id?{outcomeId:outcome.id}:{}),
   message,recovery:'Correct this declaration with ./docs route --input FILE; preserve other routes and source-backed prose.'});
 };
 problem(Array.isArray(routes),'ROUTE_LIST_REQUIRED','/routes','array',null,null,null,'expected routes array');
 for(const [ri,r] of (Array.isArray(routes)?routes:[]).entries()){
  const p='/routes/'+ri;
  if(!r||typeof r!=='object'){problem(false,'ROUTE_OBJECT_REQUIRED',p,'object',r,null,null,'expected route object');continue}
  const occurrences=Array.isArray(r.occurrences)?r.occurrences:[];
  problem(Array.isArray(r.occurrences),'ROUTE_OCCURRENCES_REQUIRED',p+'/occurrences','array',r.occurrences,r,null,'expected occurrence array');
  const ids=occurrences.map(o=>o?.id),known=new Set(ids);
  problem(known.size===ids.length,'ROUTE_OCCURRENCE_DUPLICATE',p+'/occurrences','unique occurrence IDs',ids,r,null,'duplicate occurrence ID');
  problem(['known','unknown'].includes(r.origin?.status),'ROUTE_ORIGIN_STATUS_INVALID',p+'/origin/status',['known','unknown'],r.origin?.status,r,null,'invalid origin status');
  if(r.origin?.status==='known'){
   problem(triggerKinds.includes(r.origin.kind),'ROUTE_TRIGGER_KIND_INVALID',p+'/origin/kind',triggerKinds,r.origin.kind,r,null,'invalid trigger kind');
   problem(occurrences.some(o=>o?.id===r.origin.occurrence&&o.actor?.kind==='service'),'ROUTE_ORIGIN_UNKNOWN',p+'/origin/occurrence','declared service occurrence',r.origin.occurrence,r,null,'origin must name a service occurrence');
  }
  const branches=Array.isArray(r.branches)?r.branches:[],branchIds=branches.map(b=>b?.id);
  problem(Array.isArray(r.branches)&&new Set(branchIds).size===branchIds.length,'ROUTE_BRANCHES_INVALID',p+'/branches','array of unique branch IDs',r.branches,r,null,'missing or duplicate branches');
  const outcomes=Array.isArray(r.outcomes)?r.outcomes:[];
  problem(Array.isArray(r.outcomes),'ROUTE_OUTCOMES_REQUIRED',p+'/outcomes','array',r.outcomes,r,null,'expected outcomes array');
  for(const [oi,o] of outcomes.entries()){
   const q=p+'/outcomes/'+oi;
   if(!o||typeof o!=='object'){problem(false,'ROUTE_OUTCOME_REQUIRED',q,'object',o,r,null,'expected outcome object');continue}
   problem(outcomeKinds.includes(o.kind),'ROUTE_OUTCOME_KIND_INVALID',q+'/kind',outcomeKinds,o.kind,r,o,'invalid outcome kind');
   problem(branchIds.includes(o.branch),'ROUTE_BRANCH_UNKNOWN',q+'/branch',branchIds,o.branch,r,o,'unknown branch');
   problem(['complete-for-this-outcome','partial','unknown'].includes(o.internal?.coverage),'ROUTE_INTERNAL_COVERAGE_INVALID',q+'/internal/coverage',['complete-for-this-outcome','partial','unknown'],o.internal?.coverage,r,o,'invalid internal coverage');
   const internal=o.internal?.occurrences,rows=Array.isArray(internal)?internal:[];
   problem(Array.isArray(internal),'ROUTE_INTERNAL_REQUIRED',q+'/internal/occurrences','array',internal,r,o,'expected internal occurrence array');
   problem(new Set(rows).size===rows.length&&rows.every(id=>known.has(id)),'ROUTE_INTERNAL_UNKNOWN',q+'/internal/occurrences','unique declared occurrence IDs',rows,r,o,'duplicate or unknown internal occurrence');
   problem(r.origin?.status!=='known'||!rows.includes(r.origin.occurrence),'ROUTE_ORIGIN_INTERNAL',q+'/internal/occurrences','excludes origin occurrence',rows,r,o,'origin occurrence is not internal');
   problem(['known','unknown'].includes(o.end?.status),'ROUTE_END_STATUS_INVALID',q+'/end/status',['known','unknown'],o.end?.status,r,o,'invalid end status');
   if(o.end?.status==='known'){
    const owner=o.end.ownerOccurrence;
    problem(known.has(owner),'ROUTE_TERMINAL_UNKNOWN',q+'/end/ownerOccurrence',ids,owner,r,o,'unknown terminal occurrence');
    problem(!rows.includes(owner),'ROUTE_TERMINAL_INTERNAL',q+'/internal/occurrences',{'excludesTerminal':owner},rows,r,o,'terminal occurrence is not internal');
    problem(['return','publish','handoff','stop','other'].includes(o.end.boundary),'ROUTE_BOUNDARY_INVALID',q+'/end/boundary',['return','publish','handoff','stop','other'],o.end.boundary,r,o,'invalid end boundary');
   }
  }
  problem(['complete-for-declared-scope','partial','unknown'].includes(r.coverage?.outcomeInventory),'ROUTE_COVERAGE_INVALID',p+'/coverage/outcomeInventory',['complete-for-declared-scope','partial','unknown'],r.coverage?.outcomeInventory,r,null,'invalid inventory coverage');
 }
 if(issues.length)throw Object.assign(Error('Invalid docset routes '+issues[0].field+': '+issues[0].message),
  {code:'ROUTE_DECLARATION_INVALID',details:{issues,recovery:'Use ./docs route --input FILE for only the named route declarations. No page rewrite is required.'}});
 return {status:'ROUTE_STRUCTURE_CHECKED',routes:routes.length};
}
