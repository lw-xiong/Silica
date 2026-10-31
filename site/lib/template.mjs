import { escapeHtml } from './highlight.mjs';

// The top bar's call to action. lapstack's is its browser app; a site whose
// product is a download says so instead. A site with neither gets no button.
const ctaOf = (site) => site.cta
  || (site.appUrl ? { text: `Open ${site.title}`, url: site.appUrl } : null);

const abs = (site, url) => {
  if (/^[a-z]+:/i.test(url)) return url;
  const base = site.baseUrl.endsWith('/') ? site.baseUrl : `${site.baseUrl}/`;
  return base + String(url).replace(/^\//, '');
};

// A nav entry is "current" for its own URL, and an ancestor of the current page
// is marked too, so the section a reader is inside stays lit in the sidebar.
const isCurrent = (url, pageUrl) => abs0(url) === abs0(pageUrl);
const abs0 = (u) => `/${String(u).replace(/^\/+/, '').replace(/\/+$/, '')}/`.replace('//', '/');

function topNav(site, pageUrl) {
  // An entry may carry a class of its own (the company page's login link),
  // which joins "on" when the entry is current.
  const links = site.nav.map((n) => {
    const here = abs0(pageUrl).startsWith(abs0(n.url)) && abs0(n.url) !== '/';
    const cls = [n.class, here ? 'on' : ''].filter(Boolean).join(' ');
    return `<a href="${abs(site, n.url)}"${cls ? ` class="${escapeHtml(cls)}"` : ''}${here ? ' aria-current="page"' : ''}>${escapeHtml(n.text)}</a>`;
  }).join('');
  return links;
}

function sidebar(site, pageUrl) {
  if (!site.sidebar) return '';
  const groups = site.sidebar.map((g) => {
    const items = g.items.map((it) => {
      const here = isCurrent(it.url, pageUrl);
      return `<li><a href="${abs(site, it.url)}"${here ? ' class="on" aria-current="page"' : ''}>${escapeHtml(it.text)}</a></li>`;
    }).join('');
    return `<div class="side-group"><h4>${escapeHtml(g.title)}</h4><ul>${items}</ul></div>`;
  }).join('');
  // The sidebar doubles as the small-screen drawer, so it carries a copy of the
  // main nav (hidden above the breakpoint) — otherwise Pricing and Download are
  // unreachable from a manual page on a phone.
  const cta = ctaOf(site);
  const mobile = `<div class="side-group side-main"><h4>${escapeHtml(site.title)}</h4><ul>` +
    site.nav.map((n) => `<li><a href="${abs(site, n.url)}">${escapeHtml(n.text)}</a></li>`).join('') +
    (cta ? `<li><a href="${abs(site, cta.url)}">${escapeHtml(cta.text)} \u2197</a></li>` : '') +
    `</ul></div>`;
  return `<nav class="sidebar" id="sidebar" aria-label="Manual">${mobile}${groups}</nav>`;
}

function tocRail(toc) {
  if (!toc || toc.length < 2) return '';
  const items = toc.map((h) =>
    `<li class="d${h.depth}"><a href="#${h.id}">${escapeHtml(h.text)}</a></li>`).join('');
  return `<aside class="toc" aria-label="On this page"><h4>On this page</h4><ul>${items}</ul></aside>`;
}

// Prev/next come from the flattened sidebar, so the manual reads as one
// sequence for anyone going through it front to back.
function pager(site, pageUrl) {
  if (!site.sidebar) return '';
  const flat = site.sidebar.flatMap((g) => g.items);
  const i = flat.findIndex((it) => isCurrent(it.url, pageUrl));
  if (i < 0) return '';
  const prev = flat[i - 1], next = flat[i + 1];
  const cell = (it, dir, label) => it
    ? `<a class="pg ${dir}" href="${abs(site, it.url)}"><span class="dim small">${label}</span><b>${escapeHtml(it.text)}</b></a>`
    : '<span></span>';
  return `<nav class="pager">${cell(prev, 'prev', '← Previous')}${cell(next, 'next', 'Next →')}</nav>`;
}

export function page({ site, url, title, description, html, toc, layout = 'doc', editPath, noindex = false }) {
  const isDoc = layout === 'doc';
  // The company page is a few static pages with no manual: no search, no theme
  // switch (it follows the system), no app button. Its shell is the wordmark and
  // three links, so that the typography is the page.
  const company = site.shell === 'company';
  const cta = ctaOf(site);
  const themeKey = `${site.name}-theme`;
  // A site with a brand mark puts it in the wordmark; one without is the
  // wordmark alone, as all three sites were before silica had a logo.
  const mark = site.mark
    ? `<img class="wordmark-mark" src="${abs(site, site.mark)}" alt="" aria-hidden="true">`
    : '';
  const styles = (site.styles || []).map((h) => `\n<link rel="stylesheet" href="${abs(site, h)}">`).join('');
  const scripts = (site.scripts || []).map((s) => `\n<script src="${abs(site, s)}" defer></script>`).join('');
  const full = title === site.title ? site.title : `${title} · ${site.title}`;
  const canonical = `https://${site.host}${abs0(url)}`.replace(/\/$/, url === '/' ? '/' : '/');
  const desc = description || site.description;

  return `<!doctype html>
<html lang="en" data-layout="${layout}" data-site="${escapeHtml(site.name)}" data-theme-key="${escapeHtml(themeKey)}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escapeHtml(full)}</title>
<meta name="description" content="${escapeHtml(desc)}">${noindex ? '\n<meta name="robots" content="noindex">' : ''}
<link rel="canonical" href="${escapeHtml(canonical)}">
<meta property="og:type" content="website">
<meta property="og:site_name" content="${escapeHtml(site.title)}">
<meta property="og:title" content="${escapeHtml(full)}">
<meta property="og:description" content="${escapeHtml(desc)}">
<meta property="og:url" content="${escapeHtml(canonical)}">
<meta name="twitter:card" content="summary">
<link rel="icon" href="${abs(site, '/assets/favicon.svg')}" type="image/svg+xml">
<link rel="stylesheet" href="${abs(site, '/assets/site.css')}">${styles}${company ? '' : `
<script>
// Theme before first paint, so a reader who chose light never sees a dark flash.
try { var t = localStorage.getItem('${themeKey}'); if (t) document.documentElement.dataset.theme = t; } catch (e) {}
</script>`}
</head>
<body>
<a class="skip" href="#content">Skip to content</a>
<header class="topbar">
  <div class="bar">
    <a class="wordmark" href="${abs(site, '/')}">${mark}${escapeHtml(site.title)}</a>${company ? `
    <nav class="mainnav" aria-label="Main">${topNav(site, url)}</nav>` : `
    <button class="menu-btn" id="menu-btn" aria-label="Menu" aria-expanded="false">☰</button>
    <nav class="mainnav" aria-label="Main">${topNav(site, url)}</nav>
    <div class="bar-right">
      <form class="search" role="search" id="search-form" autocomplete="off">
        <input type="search" id="search-input" placeholder="Search the manual" aria-label="Search the manual">
        <div class="results" id="search-results" hidden></div>
      </form>
      <button class="theme-btn" id="theme-btn" aria-label="Switch theme" title="Switch theme"><svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true"><circle cx="8" cy="8" r="6.2" fill="none" stroke="currentColor" stroke-width="1.5"/><path d="M8 1.8a6.2 6.2 0 0 0 0 12.4Z" fill="currentColor"/></svg></button>${cta ? `
      <a class="cta" href="${abs(site, cta.url)}">${escapeHtml(cta.text)}</a>` : ''}
    </div>`}
  </div>
</header>
<div class="shell${isDoc ? ' doc' : ''}">
  ${isDoc ? sidebar(site, url) : ''}
  <main id="content">
    <article class="prose${isDoc ? '' : ' wide'}">
${html}
    </article>
    ${isDoc ? pager(site, url) : ''}
    ${isDoc && editPath ? `<p class="dim small edit">Source: <code>${escapeHtml(editPath)}</code></p>` : ''}
  </main>
  ${isDoc ? tocRail(toc) : ''}
</div>
<footer class="footer">
  <div class="bar">
    <span class="dim small">${escapeHtml(site.footer || '')}</span>
    <nav class="foot-links">${(site.footerLinks || []).map((l) =>
      `<a href="${abs(site, l.url)}">${escapeHtml(l.text)}</a>`).join('')}</nav>
  </div>
</footer>
${company ? '' : `<script src="${abs(site, '/assets/site.js')}" defer></script>`}${scripts}
</body>
</html>
`;
}
