#!/usr/bin/env node
// Static builder for the ragtux.com sites.
//
//   node build.mjs                 build every site in sites/ into dist/
//   node build.mjs lapstack        build one site
//   node build.mjs --serve         build, then serve dist/<site> on :4321
//   node build.mjs --serve --watch rebuild on every change
//
// One site is a directory under sites/: a site.json, a content/ tree of .md
// and .html pages, and an optional assets/ directory. Everything in shared/ is
// copied into every site's /assets.

import { readFile, writeFile, mkdir, readdir, stat, cp, rm } from 'node:fs/promises';
import { existsSync, watch } from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import { fileURLToPath } from 'node:url';
import { frontMatter, render } from './lib/markdown.mjs';
import { page } from './lib/template.mjs';

const ROOT = path.dirname(fileURLToPath(import.meta.url));
const SITES = path.join(ROOT, 'sites');
const DIST = path.join(ROOT, 'dist');

const argv = process.argv.slice(2);
const flags = new Set(argv.filter((a) => a.startsWith('--')));
const named = argv.filter((a) => !a.startsWith('--'));

async function walk(dir, base = dir) {
  const out = [];
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) out.push(...await walk(full, base));
    else out.push(path.relative(base, full));
  }
  return out;
}

// content/docs/app/fusion.md  ->  url /docs/app/fusion/  file docs/app/fusion/index.html
// content/index.md            ->  url /               file index.html
function routeOf(rel) {
  const noExt = rel.replace(/\.(md|html)$/i, '');
  if (noExt === 'index') return { url: '/', file: 'index.html' };
  if (noExt.endsWith('/index')) {
    const d = noExt.slice(0, -'/index'.length);
    return { url: `/${d}/`, file: `${d}/index.html` };
  }
  return { url: `/${noExt}/`, file: `${noExt}/index.html` };
}

async function buildSite(name) {
  const dir = path.join(SITES, name);
  const site = JSON.parse(await readFile(path.join(dir, 'site.json'), 'utf8'));
  site.baseUrl ||= '/';
  const out = path.join(DIST, name);
  await rm(out, { recursive: true, force: true });
  await mkdir(out, { recursive: true });

  // assets: shared first, then the site's own, so a site can override
  await cp(path.join(ROOT, 'shared'), path.join(out, 'assets'), { recursive: true });
  if (existsSync(path.join(dir, 'assets'))) {
    await cp(path.join(dir, 'assets'), path.join(out, 'assets'), { recursive: true });
  }
  // static/: files that belong at the site's root as they are (.well-known/,
  // a verification file), not under /assets and not rendered.
  if (existsSync(path.join(dir, 'static'))) {
    await cp(path.join(dir, 'static'), out, { recursive: true });
  }

  const contentDir = path.join(dir, 'content');
  const files = (await walk(contentDir)).filter((f) => /\.(md|html)$/i.test(f)).sort();
  const index = [];
  let count = 0;

  for (const rel of files) {
    const src = await readFile(path.join(contentDir, rel), 'utf8');
    const { data, body } = frontMatter(src);
    const { url, file } = routeOf(rel.split(path.sep).join('/'));
    const isHtml = /\.html$/i.test(rel);

    const { html, toc, text } = isHtml
      ? { html: body, toc: [], text: body.replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ').trim() }
      : render(body, { baseUrl: site.baseUrl, pageUrl: url });

    const title = data.title || (/^#\s+(.+)$/m.exec(body)?.[1] ?? name);
    const layout = data.layout || (url.startsWith('/docs') ? 'doc' : 'page');

    const out_html = page({
      site, url, title,
      description: data.description,
      html, toc, layout,
      editPath: `sites/${name}/content/${rel.split(path.sep).join('/')}`,
      noindex: data.noindex === true,
    });

    const dest = path.join(out, file);
    await mkdir(path.dirname(dest), { recursive: true });
    await writeFile(dest, out_html);
    count += 1;

    if (data.noindex !== true) {
      index.push({
        u: url,
        t: title,
        d: data.description || '',
        // Headings are indexed separately from the body so a hit on a section
        // title can outrank a passing mention in prose.
        h: toc.map((x) => x.text),
        b: text.slice(0, 8000),
      });
    }
  }

  await writeFile(path.join(out, 'search-index.json'), JSON.stringify(index));

  const urls = index.map((p) => `  <url><loc>https://${site.host}${p.u}</loc></url>`).join('\n');
  await writeFile(path.join(out, 'sitemap.xml'),
    `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${urls}\n</urlset>\n`);
  await writeFile(path.join(out, 'robots.txt'),
    `User-agent: *\nAllow: /\nSitemap: https://${site.host}/sitemap.xml\n`);

  // 404: the template with a short body, served by the host for unknown paths.
  await writeFile(path.join(out, '404.html'), page({
    site, url: '/404', title: 'Page not found', layout: 'page',
    description: 'That page is not here.',
    html: `<h1>Page not found</h1>\n${site.notFound || `<p>That address is not part of this site. The <a href="${site.baseUrl}docs/">manual's contents</a> is a good place to pick the thread back up.</p>`}`,
    toc: [],
  }));

  const bytes = (await walk(out)).length;
  console.log(`${name}: ${count} pages, ${bytes} files -> dist/${name}`);
  await checkLinks(out, name, site);
  return site;
}

// Every internal href must resolve to something in dist/, and every in-page
// anchor to an id on the page it points at. A manual is mostly cross-references;
// a broken one is the failure readers notice first and authors notice last.
async function checkLinks(out, name, site) {
  const files = (await walk(out)).filter((f) => f.endsWith('.html'));
  const ids = new Map();     // page url -> Set of ids
  const pages = new Set();
  for (const f of files) {
    const html = await readFile(path.join(out, f), 'utf8');
    const url = `/${f.replace(/index\.html$/, '').split(path.sep).join('/')}`;
    pages.add(url);
    ids.set(url, new Set([...html.matchAll(/\sid="([^"]+)"/g)].map((m) => m[1])));
  }
  const base = site.baseUrl.endsWith('/') ? site.baseUrl : `${site.baseUrl}/`;
  const problems = [];
  for (const f of files) {
    const html = await readFile(path.join(out, f), 'utf8');
    const from = `/${f.replace(/index\.html$/, '').split(path.sep).join('/')}`;
    for (const m of html.matchAll(/href="([^"]+)"/g)) {
      const href = m[1];
      if (/^(https?:|mailto:|#)/.test(href) || !href.startsWith('/')) continue;
      const [p0, frag] = href.split('#');
      const target = p0.startsWith(base) ? `/${p0.slice(base.length)}` : p0;
      const onDisk = existsSync(path.join(out, target.replace(/^\//, ''), 'index.html'))
        || existsSync(path.join(out, target.replace(/^\//, '')));
      if (!pages.has(target) && !onDisk) { problems.push(`${from} -> ${href} (no such page)`); continue; }
      if (frag && ids.has(target) && !ids.get(target).has(frag)) {
        problems.push(`${from} -> ${href} (no #${frag} on that page)`);
      }
    }
  }
  if (problems.length) {
    console.error(`${name}: ${problems.length} broken link(s)`);
    for (const p of problems) console.error(`  ${p}`);
    process.exitCode = 1;
  } else {
    console.log(`${name}: internal links and anchors all resolve`);
  }
}

async function buildAll() {
  const names = named.length
    ? named
    : (await readdir(SITES, { withFileTypes: true })).filter((d) => d.isDirectory()).map((d) => d.name);
  const sites = [];
  for (const n of names) {
    try { sites.push([n, await buildSite(n)]); }
    catch (e) { console.error(`${n}: ${e.message}`); process.exitCode = 1; }
  }
  return sites;
}

const MIME = {
  '.html': 'text/html; charset=utf-8', '.css': 'text/css; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8', '.json': 'application/json; charset=utf-8',
  '.svg': 'image/svg+xml', '.png': 'image/png', '.jpg': 'image/jpeg', '.webp': 'image/webp',
  '.woff2': 'font/woff2', '.xml': 'application/xml; charset=utf-8', '.txt': 'text/plain; charset=utf-8',
};

function serve(siteName, port = 4321) {
  const root = path.join(DIST, siteName);
  http.createServer(async (req, res) => {
    let p = decodeURIComponent(req.url.split('?')[0]);
    if (p.endsWith('/')) p += 'index.html';
    let file = path.join(root, p);
    if (!file.startsWith(root)) { res.writeHead(403).end(); return; }
    try {
      if ((await stat(file)).isDirectory()) file = path.join(file, 'index.html');
    } catch {
      // A path with no extension is a page whose directory was not typed with
      // its trailing slash; try it before giving up.
      if (!path.extname(file)) file = path.join(file, 'index.html');
    }
    try {
      const body = await readFile(file);
      res.writeHead(200, { 'Content-Type': MIME[path.extname(file)] || 'application/octet-stream', 'Cache-Control': 'no-store' });
      res.end(body);
    } catch {
      try {
        const body = await readFile(path.join(root, '404.html'));
        res.writeHead(404, { 'Content-Type': MIME['.html'] }).end(body);
      } catch { res.writeHead(404).end('not found'); }
    }
  }).listen(port, '127.0.0.1', () => console.log(`serving dist/${siteName} on http://localhost:${port}/`));
}

const built = await buildAll();

if (flags.has('--serve')) {
  const first = named[0] || built[0]?.[0];
  if (!first) { console.error('nothing to serve'); process.exit(1); }
  serve(first, Number(process.env.PORT) || 4321);
  if (flags.has('--watch')) {
    let timer = null;
    for (const d of ['sites', 'shared', 'lib']) {
      watch(path.join(ROOT, d), { recursive: true }, () => {
        clearTimeout(timer);
        timer = setTimeout(() => buildAll().catch((e) => console.error(e.message)), 120);
      });
    }
    console.log('watching sites/, shared/, lib/');
  }
}
