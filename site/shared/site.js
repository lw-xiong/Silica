/* ragtux.com sites — the whole client side. No framework, no build step. */
(() => {
  'use strict';
  const $ = (s, r = document) => r.querySelector(s);
  // The script lives at <base>assets/site.js, which is how the page's base URL
  // is recovered without templating it in.
  const BASE = (document.currentScript?.src || '').replace(/assets\/site\.js.*$/, '') || '/';

  /* ---------- theme ---------- */
  const themeBtn = $('#theme-btn');
  if (themeBtn) {
    const systemDark = () => window.matchMedia('(prefers-color-scheme: dark)').matches;
    themeBtn.addEventListener('click', () => {
      const cur = document.documentElement.dataset.theme || (systemDark() ? 'dark' : 'light');
      const next = cur === 'dark' ? 'light' : 'dark';
      document.documentElement.dataset.theme = next;
      // The key is per site (data-theme-key on <html>), so a reader's choice on
      // one of our sites is not also a choice on the next.
      const key = document.documentElement.dataset.themeKey || 'ragtux-theme';
      try { localStorage.setItem(key, next); } catch (e) {}
    });
  }

  /* ---------- mobile menu ---------- */
  const menuBtn = $('#menu-btn'), nav = $('.mainnav'), side = $('#sidebar');
  if (menuBtn) {
    // On a manual page the drawer is the sidebar (which carries a copy of the
    // main nav for small screens); elsewhere there is no sidebar and the drawer
    // is the main nav itself.
    const drawer = side || nav;
    menuBtn.addEventListener('click', () => {
      const open = !drawer.classList.contains('open');
      drawer.classList.toggle('open', open);
      menuBtn.setAttribute('aria-expanded', String(open));
      menuBtn.textContent = open ? '\u2715' : '\u2630';
    });
  }

  /* ---------- on-this-page: highlight the section in view ---------- */
  const tocLinks = [...document.querySelectorAll('.toc a')];
  if (tocLinks.length) {
    const byId = new Map(tocLinks.map((a) => [a.getAttribute('href').slice(1), a]));
    const heads = [...document.querySelectorAll('.prose h2[id], .prose h3[id]')].filter((h) => byId.has(h.id));
    // Track which headings have scrolled past the top band; the last one that
    // has is the section being read. An IntersectionObserver alone would light
    // up whichever heading is merely visible, which reads wrong when a long
    // section fills the window with no heading in it at all.
    const seen = new Set();
    const mark = () => {
      let cur = null;
      for (const h of heads) if (seen.has(h.id)) cur = h.id;
      for (const [id, a] of byId) a.classList.toggle('on', id === cur);
    };
    const io = new IntersectionObserver((entries) => {
      for (const e of entries) {
        if (e.boundingClientRect.top < 90) { if (e.isIntersecting) seen.delete(e.target.id); else seen.add(e.target.id); }
        else seen.delete(e.target.id);
      }
      mark();
    }, { rootMargin: '-80px 0px -75% 0px', threshold: 0 });
    heads.forEach((h) => io.observe(h));
  }

  /* ---------- search ---------- */
  const form = $('#search-form'), input = $('#search-input'), box = $('#search-results');
  if (form && input && box) {
    let index = null, loading = null, sel = -1, rows = [];

    const load = () => (loading ||= fetch(`${BASE}search-index.json`)
      .then((r) => r.json())
      .then((j) => { index = j; return j; })
      .catch(() => { index = []; return []; }));

    const norm = (s) => s.toLowerCase().normalize('NFKD').replace(/[̀-ͯ]/g, '');
    const esc = (s) => s.replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));

    function score(page, terms) {
      const t = norm(page.t), d = norm(page.d), b = norm(page.b);
      const hs = page.h.map(norm);
      let total = 0;
      for (const term of terms) {
        let s = 0;
        if (t.includes(term)) s += t.startsWith(term) ? 16 : 10;
        if (hs.some((h) => h.includes(term))) s += 5;
        if (d.includes(term)) s += 3;
        const n = b.split(term).length - 1;
        if (n) s += Math.min(4, 1 + Math.log2(n));
        if (!s) return 0;           // every term must appear somewhere
        total += s;
      }
      return total;
    }

    function snippet(page, terms) {
      const b = page.b, lb = norm(b);
      let at = -1;
      for (const term of terms) { const i = lb.indexOf(term); if (i >= 0 && (at < 0 || i < at)) at = i; }
      if (at < 0) return esc(page.d || b.slice(0, 140));
      const from = Math.max(0, at - 60), to = Math.min(b.length, at + 120);
      let s = esc((from ? '…' : '') + b.slice(from, to) + (to < b.length ? '…' : ''));
      for (const term of terms) {
        s = s.replace(new RegExp(`(${term.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')})`, 'ig'), '<mark>$1</mark>');
      }
      return s;
    }

    function run(q) {
      const terms = norm(q).split(/\s+/).filter((w) => w.length > 1);
      if (!terms.length || !index) { close(); return; }
      const hits = index
        .map((p) => ({ p, s: score(p, terms) }))
        .filter((x) => x.s > 0)
        .sort((a, b) => b.s - a.s)
        .slice(0, 8);
      if (!hits.length) {
        box.innerHTML = `<div class="none">Nothing matched “${esc(q)}”.</div>`;
        box.hidden = false; rows = []; sel = -1; return;
      }
      box.innerHTML = hits.map(({ p }) =>
        `<a href="${p.u}"><b>${esc(p.t)}</b><span>${snippet(p, terms)}</span></a>`).join('');
      box.hidden = false;
      rows = [...box.querySelectorAll('a')];
      sel = -1;
    }

    const close = () => { box.hidden = true; rows = []; sel = -1; };
    const move = (d) => {
      if (!rows.length) return;
      sel = sel < 0 ? (d > 0 ? 0 : rows.length - 1) : (sel + d + rows.length) % rows.length;
      rows.forEach((a, i) => a.classList.toggle('sel', i === sel));
      rows[sel].scrollIntoView({ block: 'nearest' });
    };

    let timer = null;
    input.addEventListener('input', () => {
      clearTimeout(timer);
      const q = input.value.trim();
      if (!q) { close(); return; }
      timer = setTimeout(() => load().then(() => run(q)), 90);
    });
    input.addEventListener('focus', load);
    form.addEventListener('submit', (e) => { e.preventDefault(); if (rows[Math.max(sel, 0)]) location.href = rows[Math.max(sel, 0)].href; });
    input.addEventListener('keydown', (e) => {
      if (e.key === 'ArrowDown') { e.preventDefault(); move(1); }
      else if (e.key === 'ArrowUp') { e.preventDefault(); move(-1); }
      else if (e.key === 'Escape') { close(); input.blur(); }
    });
    document.addEventListener('click', (e) => { if (!form.contains(e.target)) close(); });
    document.addEventListener('keydown', (e) => {
      if (e.key === '/' && !/^(INPUT|TEXTAREA|SELECT)$/.test(document.activeElement?.tagName)) {
        e.preventDefault(); input.focus(); input.select();
      }
    });
  }
})();
