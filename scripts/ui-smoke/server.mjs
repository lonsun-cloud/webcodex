import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { webResponse, desktopState, adminDashboard } from './fixtures.mjs';
const directory = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(directory, '../..');

// Test-only loopback server. Never proxy requests or invoke a native command.
export async function startFixtureServer() {
  const requests = [];
  const server = http.createServer(async (req, res) => {
    const url = new URL(req.url, 'http://127.0.0.1');
    const send = (status, body, type = 'application/json') => {
      res.writeHead(status, { 'Content-Type': type, 'Cache-Control': 'no-store', 'X-UI-Fixture': 'isolated' });
      res.end(typeof body === 'string' || Buffer.isBuffer(body) ? body : JSON.stringify(body));
    };
    try {
      if (url.pathname === '/favicon.ico') return send(204, '');
      if (url.pathname === '/__fixture/desktop-state') return send(200, desktopState(url.searchParams.get('state') === 'disconnected'));
      if (url.pathname === '/__fixture/requests') return send(200, requests);
      if (url.pathname === '/__fixture/health') return send(200, { fixture: true, pid: process.pid });
      if (url.pathname === '/api/admin/dashboard' && req.method === 'POST') return send(200, adminDashboard);
      if (url.pathname.startsWith('/api/runtime-console/')) {
        let body = '';
        for await (const chunk of req) { body += chunk; if (body.length > 65536) return send(413, { error: 'Fixture request bound' }); }
        const payload = JSON.parse(body || '{}'), route = url.pathname.slice('/api/runtime-console/'.length);
        requests.push({ route, payload }); if (requests.length > 300) requests.shift();
        if (route === 'workflow-session-observe' && payload.observation_token) await new Promise(resolve => setTimeout(resolve, 1000));
        return send(200, webResponse(route, payload));
      }
      if (url.pathname === '/') return send(200, '<!doctype html><title>Isolated UI fixtures</title><h1>Fixture only — no native backend</h1><a href="/runtime/">WebUI</a> · <a href="/desktop/">Desktop renderer</a> · <a href="/admin/">Admin</a>', 'text/html');
      if (url.pathname === '/desktop-shim.js') return send(200, fs.readFileSync(path.join(directory, 'desktop-shim.js')), 'text/javascript');
      const desktop = url.pathname.startsWith('/desktop/') || url.pathname.startsWith('/assets/');
      const admin = url.pathname.startsWith('/admin/');
      let relative = desktop ? (url.pathname.replace(/^\/desktop\//, '') || 'index.html') : admin ? url.pathname.replace(/^\/admin\/?/, '') : url.pathname.replace(/^\/runtime\/?/, '');
      if (admin) relative = ({ '': 'admin.html', 'app.js': 'admin.js', 'styles.css': 'admin.css' })[relative] || relative;
      else if (!desktop) relative = ({ '': 'runtime.html', 'app.js': 'app.js', 'styles.css': 'styles.css' })[relative] || relative;
      if (relative.startsWith('/assets/')) relative = relative.slice(1);
      const base = path.join(root, desktop ? 'apps/desktop/dist' : 'frontend/dist');
      const file = path.resolve(base, relative);
      if (!file.startsWith(base + path.sep)) return send(403, { error: 'Fixture path denied' });
      if (!fs.existsSync(file)) return send(404, { error: 'Build both renderers before running this fixture' });
      let bytes = fs.readFileSync(file);
      const extension = path.extname(file);
      const type = ({ '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.png': 'image/png', '.svg': 'image/svg+xml' })[extension] || 'application/octet-stream';
      if (extension === '.html') {
        let html = bytes.toString().replace('<title>', '<title>[ISOLATED FIXTURE] ');
        if (desktop) html = html.replace(/<script type="module"[^>]*src="([^"]+)"[^>]*><\/script>/, (_, src) => `<script src="/desktop-shim.js"></script><script type="module">await window.__fixtureReady;await import(${JSON.stringify(src)});</script>`);
        else html = html.replace('<head>', '<head><script>sessionStorage.setItem("webcodex.runtime.credential.v1","fixture-only-not-a-secret");localStorage.setItem("webcodex.runtime.language.v1","en");</script>');
        bytes = html;
      }
      send(200, bytes, type);
    } catch (error) { send(500, { fixture: true, error: error.message }); }
  });
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  const timer = setTimeout(() => server.close(), 30 * 60 * 1000); timer.unref();
  return { server, url: `http://127.0.0.1:${server.address().port}`, requests };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const { server, url } = await startFixtureServer();
  console.log(JSON.stringify({ fixture: true, pid: process.pid, url }));
  process.on('SIGTERM', () => server.close());
}
