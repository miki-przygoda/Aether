// Global helpers used across pages.
// Each page's inline <script> handles its own logic.
'use strict';

// Flash a banner at the top of the page.
function flashBanner(msg, type) {
  const el = document.createElement('div');
  el.className = 'alert alert-' + (type || 'success');
  el.textContent = msg;
  el.style.cssText = 'position:fixed;top:1rem;right:1rem;z-index:999;max-width:320px';
  document.body.appendChild(el);
  setTimeout(() => el.remove(), 3500);
}

// POST JSON and return parsed response.
async function postJSON(url, body) {
  const resp = await fetch(url, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
  const data = await resp.json().catch(() => ({}));
  return { ok: resp.ok, status: resp.status, data };
}
