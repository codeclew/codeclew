import React, {useEffect, useRef, useState} from 'react';
import useBaseUrl from '@docusaurus/useBaseUrl';

const cache = new Map();
async function jsonAt(url) {
  if (!cache.has(url)) cache.set(url, fetch(url).then(response => {
    if (!response.ok) throw new Error(`Saved documentation resource unavailable (${response.status})`);
    return response.json();
  }));
  return cache.get(url);
}
async function resolve(value, base, seen = new Set()) {
  if (Array.isArray(value)) return Promise.all(value.map(v => resolve(v, base, seen)));
  if (!value || typeof value !== 'object') return value;
  if (Object.keys(value).length === 1 && typeof value.$clewRef === 'string') {
    const id = value.$clewRef;
    if (!/^sha256:[a-f0-9]{64}$/.test(id) || seen.has(id)) throw new Error('Invalid saved evidence reference');
    const object = await jsonAt(`${base}clew-data/objects/${id.slice(7)}.json`);
    if (object.schema !== 'codeclew-export-object/1.0' || object.identity !== id) throw new Error('Saved evidence identity differs');
    return resolve(object.value, base, new Set([...seen, id]));
  }
  const literal = Object.keys(value).length === 1 && Object.hasOwn(value, '$clewLiteral');
  const entries = Object.entries(literal ? value.$clewLiteral : value);
  return Object.fromEntries(await Promise.all(entries.map(async ([k, v]) => [k, await resolve(v, base, seen)])));
}
async function documentAt(path, base) {
  const document = await jsonAt(base + path.replace(/^\//, ''));
  if (document.schema !== 'codeclew-normalized-data/1.0') throw new Error('Unsupported saved documentation format');
  return resolve(document.value, base);
}

export function ClewDownload({path, children}) {
  return <a href={useBaseUrl(path)} download>{children}</a>;
}
export function ClewModelLink({model, process, anchor, children}) {
  const path = useBaseUrl(`/data-model/${model}`);
  const query = new URLSearchParams({fromProcess: process, fromAnchor: anchor});
  return <a data-dm-context data-dm-return-anchor={anchor} href={`${path}?${query}`}>{children}</a>;
}
export function ClewFieldTrace({field}) {
  return <a data-dm-context data-dm-field={field} href={`?field=${encodeURIComponent(field)}#field-usage`}>Trace this field</a>;
}

function Widget({dataPath, anchor}) {
  const base = useBaseUrl('/');
  const target = useRef(null);
  const [error, setError] = useState('');
  const [ready, setReady] = useState(false);
  useEffect(() => {
    let disposed = false;
    const removals = [];
    setReady(false); setError('');
    Promise.all([documentAt(dataPath, base), import('./clew-model-runtime.js')]).then(([saved]) => {
      if (disposed || !target.current) return;
      const data = {...saved};
      data.modelHrefs = Object.fromEntries((data.models || []).map(m => [m.id, `${base}data-model/${m.id}`]));
      const query = new URLSearchParams(location.search);
      const process = data.request?.subject?.slice(9) || query.get('fromProcess');
      if (process && /^[a-zA-Z0-9_-]{1,160}$/.test(process)) data.processHrefs = {[process]: `${base}scenarios/${process}`};
      if (anchor) data.returnAnchor = anchor;
      const handle = window.CodeclewDataModel.mount(target.current, data);
      document.querySelectorAll('[data-dm-field]').forEach(link => {
        const click = event => {
          if (!handle || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
          event.preventDefault(); handle.selectField(link.dataset.dmField);
          if (location.hash !== '#field-usage') history.pushState(null, '', '#field-usage');
          document.getElementById('field-usage')?.scrollIntoView({block: 'start'});
        };
        link.addEventListener('click', click); removals.push(() => link.removeEventListener('click', click));
      });
      setReady(true);
    }).catch(reason => { if (!disposed) setError(reason.message); });
    return () => { disposed = true; removals.forEach(remove => remove()); };
  }, [dataPath, base, anchor]);
  return <div className="dm-page clew-widget">
    {!ready && !error && <p role="status">Loading saved field evidence…</p>}
    {error && <p role="alert">{error}. The original prose and source links remain available.</p>}
    <div ref={target} />
  </div>;
}

export function ClewDataModel({dataPath}) {
  return <section id="field-usage"><h2>Field usage and focused flow</h2><Widget dataPath={dataPath} /></section>;
}
export function ClewProcess({dataPath, children}) {
  const [flow, setFlow] = useState(false);
  const [anchor, setAnchor] = useState('');
  useEffect(() => {
    setFlow(new URLSearchParams(location.search).get('view') === 'data-flow');
    if (/^#question-[a-zA-Z0-9_-]+$/.test(location.hash)) setAnchor(location.hash.slice(1));
  }, [dataPath]);
  function select(on) {
    if (on && /^#question-[a-zA-Z0-9_-]+$/.test(location.hash)) setAnchor(location.hash.slice(1));
    const url = new URL(location.href); url.searchParams.set('view', on ? 'data-flow' : 'process');
    history.replaceState(null, '', url); setFlow(on);
  }
  return <section className="clew-process">
    <div role="tablist" aria-label="Process reading view" className="clew-tabs">
      {[false, true].map(on => <button key={String(on)} type="button" role="tab" aria-selected={flow === on}
        aria-controls={on ? 'clew-flow' : 'clew-process'} id={on ? 'clew-flow-tab' : 'clew-process-tab'}
        tabIndex={flow === on ? 0 : -1} onClick={() => select(on)} onKeyDown={event => {
          if (['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) {
            event.preventDefault(); const next = event.key === 'Home' ? false : event.key === 'End' ? true : !on;
            select(next); document.getElementById(next ? 'clew-flow-tab' : 'clew-process-tab')?.focus();
          }
        }}>{on ? 'Data Flow' : 'Process'}</button>)}
    </div>
    <div id="clew-process" role="tabpanel" aria-labelledby="clew-process-tab" hidden={flow}>{children}</div>
    <div id="clew-flow" role="tabpanel" aria-labelledby="clew-flow-tab" hidden={!flow}>
      <p>Agent proposal · Not reviewed. This view covers only the saved process fields and consumer stages.</p>
      <p><a href={`#${anchor}`} onClick={() => select(false)}>Return to the same process step</a></p>
      {flow && <Widget dataPath={dataPath} anchor={anchor} />}
    </div>
  </section>;
}
