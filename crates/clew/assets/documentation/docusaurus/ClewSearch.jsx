import React from 'react';
import useBaseUrl from '@docusaurus/useBaseUrl';
export default function ClewSearch() {
  return <form className="clew-search" action={useBaseUrl('/catalog')} role="search">
    <input type="search" name="q" aria-label="Search documentation metadata" placeholder="Search docs" />
    <button type="submit">Search</button>
  </form>;
}
