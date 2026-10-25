'use strict';
// The renderer supplies the selected locale and the owned label dictionary.
// Tagged templates translate only static chunks, before inserting caller data.
function clewLocale(language, messages) {
  const t = key => language === 'ru' ? messages[key] ?? key : key;
  // The legacy "in" label is contextual, not a translation of the code token
  // or a preposition inside an unrelated owned sentence. Lowercase source also
  // occurs in CSS classes and data-source-* bindings; translate that label with t().
  const codes = new Set(['in','source','revision','required','optional','unknown','unavailable','unspecified','none','alt','else','loop','opt','read','transform','write','transfer','candidate','domain','dto','message','table','field','function','CURRENT','STALE','UNVERIFIED','UNASSESSED','VERIFIED','VERIFIED_WITH_LIMITATIONS','SECTION','NOTE','PROCESS','FLOW','MAP','RULES','VIEW','CODE']);
  const pattern = new RegExp('(?<![\\p{L}\\p{N}_])(?:' + Object.keys(messages).filter(key => !codes.has(key)).sort((a,b) => b.length-a.length).map(key => key.replace(/[.*+?^${}()|[\]\\]/g,'\\$&')).join('|') + ')(?![\\p{L}\\p{N}_])','gu');
  function chromeHtml(parts, ...values) {
    return parts.reduce((out, part, index) => out + (language === 'ru' ? part.replace(pattern, key => t(key)) : part) + (index < values.length ? values[index] : ''), '');
  }
  return {t, chromeHtml};
}
