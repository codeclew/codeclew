import {language, t} from './src/components/clew-locale.js';

export default {
  title: t('Codeclew documentation'),
  i18n: {defaultLocale: language, locales: [language]},
  url: process.env.CODECLEW_DOCS_URL || 'https://example.invalid',
  baseUrl: process.env.CODECLEW_DOCS_BASE_URL || '/',
  trailingSlash: false,
  onBrokenLinks: 'throw',
  onBrokenAnchors: 'throw',
  markdown: {mermaid: true, hooks: {onBrokenMarkdownLinks: 'throw'}},
  themes: ['@docusaurus/theme-mermaid'],
  presets: [['classic', {docs: {routeBasePath: '/', sidebarPath: './sidebars.js'}, blog: false, theme: {customCss: './src/css/clew.css'}}]],
  themeConfig: {
    navbar: {title: 'Codeclew', items: [
      {to: '/', label: t('Overview'), position: 'left', exact: true},
      {to: '/catalog', label: t('Browse'), position: 'left'},
      {to: '/help', label: t('Help'), position: 'right'},
      {to: '/runbooks', label: t('Runbooks'), position: 'right'},
      {type: 'custom-clewSearch', position: 'right'},
    ]},
    colorMode: {defaultMode: 'light', disableSwitch: true},
    mermaid: {options: {securityLevel: 'strict', fontSize: 16}},
  },
};
