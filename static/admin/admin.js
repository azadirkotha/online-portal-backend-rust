// Self-hosted (the admin CSP only allows scripts from 'self'). No dependencies.
(() => {
  const body = document.body;

  // --- Mobile navigation drawer ---
  const toggle = document.getElementById('nav-toggle');
  const overlay = document.getElementById('nav-overlay');
  if (toggle && overlay) {
    const setOpen = (open) => {
      body.classList.toggle('nav-open', open);
      toggle.setAttribute('aria-expanded', String(open));
      toggle.setAttribute('aria-label', open ? 'Close menu' : 'Open menu');
      overlay.hidden = !open;
    };
    toggle.addEventListener('click', () => setOpen(!body.classList.contains('nav-open')));
    overlay.addEventListener('click', () => setOpen(false));
    document.addEventListener('keydown', (e) => { if (e.key === 'Escape') setOpen(false); });
    document.querySelectorAll('.nav a').forEach((a) => a.addEventListener('click', () => setOpen(false)));
  }

  // --- Highlight the current section in the sidebar ---
  const path = location.pathname;
  document.querySelectorAll('.nav a[data-nav]').forEach((a) => {
    const base = a.dataset.nav;
    const active = base === '/' ? path === '/' : path === base || path.startsWith(base + '/');
    if (active) a.setAttribute('aria-current', 'page');
  });

  // --- Confirm destructive actions: <form data-confirm="..."> ---
  document.addEventListener('submit', (e) => {
    const msg = e.target && e.target.dataset ? e.target.dataset.confirm : null;
    if (msg && !window.confirm(msg)) e.preventDefault();
  });
})();
