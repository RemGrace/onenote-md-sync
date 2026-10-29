import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

const FLASK = 'http://127.0.0.1:8377';

// Everything Flask owns. The bundle only ever fetches relative paths, so in dev
// the browser sees one origin (the Vite server) and the proxy forwards these.
// A plain entry is a PREFIX match, so anything whose name is also the start of a
// page's URL is anchored: bare '/run' would swallow /runs and /runs/<id>, send
// them to Flask, and hand back the built bundle instead of the dev one.
const API = ['/status.json', '/login-state.json', '/log', '/run.json', '/auth',
             '/logout', '^/run$', '/login', '/code', '/pause', '/resume',
             '/schedule', '/runs/clear', '/runs/delete'];

// Flask rejects a POST whose Origin disagrees with its Host (CSRF, second layer
// behind SameSite=Lax). Proxying moves the request between two hosts, so both
// headers are restated as the target: changeOrigin rewrites Host, and Origin is
// set alongside it, because http-proxy leaves Origin alone and the browser's
// value is still the dev server. Never relax the server check to fix dev.
const proxy = Object.fromEntries(API.map((path) => [path, {
  target: FLASK,
  changeOrigin: true,
  headers: { Origin: FLASK },
}]));

export default defineConfig({
  plugins: [react()],
  // 127.0.0.1, not Vite's default localhost: the session cookie is scoped by host,
  // so the dev server has to answer on the same host the Flask sign-in was done on,
  // or every request arrives anonymous and the SPA only ever shows 'session expired'.
  server: { host: '127.0.0.1', proxy },
});
