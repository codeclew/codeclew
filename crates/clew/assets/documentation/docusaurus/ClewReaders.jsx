import React, {useEffect, useRef, useState} from 'react';
import {flushSync} from 'react-dom';
import useBaseUrl from '@docusaurus/useBaseUrl';

const cache = new Map();
const packedObjects = new Map();
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
    const object = packedObjects.get(id);
    if (!object) throw new Error('Packed evidence reference is missing');
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
  const manifest = await jsonAt(base + 'clew-reader-packs/manifest.json');
  if (manifest.schema !== 'codeclew-reader-packs/1.0' || !manifest.widgets[path]) throw new Error('Reader transport manifest is incomplete; run npm run build');
  await Promise.all(manifest.widgets[path].packs.map(async packPath => {
    const pack = await jsonAt(base + packPath.replace(/^\//, ''));
    if (pack.schema !== 'codeclew-reader-pack/1.0') throw new Error('Unsupported saved evidence pack');
    for (const [id, record] of Object.entries(pack.objects)) {
      if (record.identity !== id || record.schema !== 'codeclew-export-object/1.0') throw new Error('Packed evidence identity differs');
      packedObjects.set(id, record);
    }
  }));
  return resolve(document.value, base);
}

export function ClewDownload({path, children}) {
  return <a href={useBaseUrl(path)} download>{children}</a>;
}
export function ClewModelLink({model, process, anchor, children}) {
  const ref = useRef(null);
  const path = useBaseUrl(`/data-model/${model}`);
  const query = new URLSearchParams({fromProcess: process, fromAnchor: anchor});
  useEffect(() => {
    if (!ref.current) return;
    const q = new URLSearchParams(location.search), url = new URL(ref.current.href);
    for (const name of ['field', 'stage', 'contract', 'cellStage']) if (q.has(name)) url.searchParams.set('from' + name[0].toUpperCase() + name.slice(1), q.get(name));
    ref.current.setAttribute('href', url.pathname + url.search);
  }, [path, process, anchor]);
  return <a ref={ref} data-dm-context data-dm-return-anchor={anchor} href={`${path}?${query}`}>{children}</a>;
}
export function ClewRelatedModelLink({model, children}) {
  const ref = useRef(null);
  const path = useBaseUrl(`/data-model/${model}`);
  useEffect(() => {
    const url = new URL(ref.current.href), q = new URLSearchParams(location.search);
    for (const name of ['fromProcess', 'fromAnchor', 'fromField', 'fromStage', 'fromContract', 'fromCellStage']) if (q.has(name)) url.searchParams.set(name, q.get(name));
    ref.current.setAttribute('href', url.pathname + url.search);
  }, [path]);
  return <a ref={ref} data-dm-context href={path}>{children}</a>;
}
export function ClewFieldTrace({field}) {
  const ref = useRef(null);
  useEffect(() => {
    const url = new URL(location.href); url.searchParams.set('field', field); url.hash = 'field-usage';
    ref.current.setAttribute('href', url.pathname + url.search + url.hash);
  }, [field]);
  return <a ref={ref} data-dm-context data-dm-field={field} href={`?field=${encodeURIComponent(field)}#field-usage`}>Trace this field</a>;
}

function Widget({dataPath, anchor, contextOnly = false}) {
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
      data.modelHrefs = Object.fromEntries([...(data.models || []), ...(data.modelReferences || [])].map(m => [m.id, `${base}data-model/${m.id}`]));
      const query = new URLSearchParams(location.search);
      const process = data.request?.subject?.slice(9) || query.get('fromProcess');
      if (process && /^[a-zA-Z0-9_-]{1,160}$/.test(process)) data.processHrefs = {[process]: `${base}scenarios/${process}`};
      if (anchor) data.returnAnchor = anchor;
      const handle = contextOnly ? null : window.CodeclewDataModel.mount(target.current, data);
      if (contextOnly) window.CodeclewDataModel.contextLinks(document, data);
      document.querySelectorAll('[data-dm-field]').forEach(link => {
        const click = event => {
          if (!handle || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
          event.preventDefault(); handle.selectField(link.dataset.dmField);
          if (location.hash !== '#field-usage') history.pushState(null, '', '#field-usage');
        };
        link.addEventListener('click', click); removals.push(() => link.removeEventListener('click', click));
      });
      setReady(true);
    }).catch(reason => { if (!disposed) setError(reason.message); });
    return () => { disposed = true; removals.forEach(remove => remove()); };
  }, [dataPath, base, anchor, contextOnly]);
  return <div className="dm-page clew-widget">
    {!ready && !error && <p role="status">Loading saved field evidence…</p>}
    {error && <p role="alert">{error}. The original prose and source links remain available.</p>}
    <div ref={target} />
  </div>;
}

export function ClewDataModel({dataPath, contextOnly = false}) {
  return <Widget dataPath={dataPath} contextOnly={contextOnly} />;
}
export function ClewProcess({dataPath, defaultAnchor, children}) {
  const [flow, setFlow] = useState(false);
  const [anchor, setAnchor] = useState(defaultAnchor || '');
  useEffect(() => {
    setFlow(new URLSearchParams(location.search).get('view') === 'data-flow');
    if (/^#question-[a-zA-Z0-9_-]+$/.test(location.hash)) setAnchor(location.hash.slice(1));
    const questionNavigation = event => {
      if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
      const link = event.target.closest?.('a[href]'); if (!link) return;
      const url = new URL(link.href, location.href);
      if (url.origin !== location.origin || url.pathname !== location.pathname || !url.hash) return;
      const target = document.getElementById(decodeURIComponent(url.hash.slice(1)));
      if (!target || !document.getElementById('clew-process')?.contains(target)) return;
      event.preventDefault(); event.stopPropagation();
      flushSync(() => {setFlow(false); if (target.id.startsWith('question-')) setAnchor(target.id);});
      const next = new URL(location.href); next.searchParams.set('view', 'process'); next.hash = url.hash;
      history.pushState(null, '', next); target.scrollIntoView({block: 'start'});
    };
    const historyNavigation = () => {
      flushSync(() => {
        setFlow(new URLSearchParams(location.search).get('view') === 'data-flow');
        if (/^#question-[a-zA-Z0-9_-]+$/.test(location.hash)) setAnchor(location.hash.slice(1));
      });
    };
    document.addEventListener('click', questionNavigation, true);
    window.addEventListener('popstate', historyNavigation);
    return () => {document.removeEventListener('click', questionNavigation, true);window.removeEventListener('popstate', historyNavigation);};
  }, [dataPath]);
  function select(on) {
    if (on && /^#question-[a-zA-Z0-9_-]+$/.test(location.hash)) setAnchor(location.hash.slice(1));
    const url = new URL(location.href); url.searchParams.set('view', on ? 'data-flow' : 'process');
    history.replaceState(null, '', url); setFlow(on);
  }
  return <section className="clew-process" data-reading-view={flow ? 'data-flow' : 'process'}>
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
      <p className="dm-origin">Agent proposal · Not reviewed · <a href={`#${anchor}`} onClick={() => select(false)}>Return to process step</a></p>
      {flow && <Widget dataPath={dataPath} anchor={anchor} />}
    </div>
  </section>;
}
