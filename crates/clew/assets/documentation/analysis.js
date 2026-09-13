function analysisEvidence(){return `<section class="gap-card"><h3>Analysis evidence</h3><p>Evidence from this publication's source check. Compiler facts enrich source interpretation; deployed behavior and narrative meaning remain separately assessed.</p>${Object.entries(D.analysisEvidence||{}).map(([id,e])=>`<h4>${esc(id)}</h4><p>${esc(e.coverage)} · provider: <strong>${esc(e.provider.status)}</strong> · ${e.mappedSymbols} mapped semantic symbols</p><p>${esc(e.provider.reason||'')} ${esc(e.provider.producer||e.extractor)} · revision ${esc(e.revision.slice(0,12))}</p>${e.provider.status==='NOT_REQUESTED'?'<p>Optional compiler enrichment was not requested for this service.</p>':''}<details><summary>Provider details and example semantic facts</summary><pre>${pretty(e)}</pre></details>`).join('')}</section>`;}
document.addEventListener('click', event => {
 if (event.target.closest('#coverage-button')) {
  document.getElementById('coverage-view').querySelector('.coverage-grid').insertAdjacentHTML('afterend', analysisEvidence());
 }
});
