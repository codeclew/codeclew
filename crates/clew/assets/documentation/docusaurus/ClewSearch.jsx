import React from 'react';
import useBaseUrl from '@docusaurus/useBaseUrl';
import {t} from './clew-locale.js';
export default function ClewSearch() {
  return <form className="clew-search" action={useBaseUrl('/catalog')} role="search">
    <input type="search" name="q" aria-label={t('Search documentation metadata')} placeholder={t('Search docs')} />
    <button type="submit">{t('Search')}</button>
  </form>;
}
