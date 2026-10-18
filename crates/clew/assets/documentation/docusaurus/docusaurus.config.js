export default {
  title: 'Codeclew documentation',
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
      {to: '/', label: 'Overview', position: 'left'},
      {to: '/catalog', label: 'Browse', position: 'left'},
      {to: '/help', label: 'Help', position: 'right'},
      {to: '/runbooks', label: 'Runbooks', position: 'right'},
      {type: 'custom-clewSearch', position: 'right'},
    ]},
    colorMode: {defaultMode: 'light', disableSwitch: true},
    mermaid: {options: {securityLevel: 'strict', fontSize: 16}},
  },
};
