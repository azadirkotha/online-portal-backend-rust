// Image upload widget used by site settings, My account and the user forms.
// Markup: <div data-image-field> with data-image-value (hidden input), data-image-preview, data-image-img,
// data-image-file, data-image-pick, data-image-remove, data-image-status. The form needs data-csrf.
(() => {
  document.querySelectorAll('[data-image-field]').forEach((box) => {
    const form = box.closest('form');
    const csrf = form ? form.dataset.csrf : '';
    const q = (name) => box.querySelector('[data-image-' + name + ']');
    const value = q('value'), preview = q('preview'), img = q('img'), file = q('file');
    const pick = q('pick'), remove = q('remove'), status = q('status');

    const show = (url) => {
      value.value = url || '';
      if (url) { img.src = url; preview.hidden = false; remove.hidden = false; }
      else { img.removeAttribute('src'); preview.hidden = true; remove.hidden = true; }
    };

    pick.addEventListener('click', () => file.click());
    remove.addEventListener('click', () => show(''));
    file.addEventListener('change', async () => {
      const f = file.files[0];
      if (!f) return;
      status.textContent = 'Uploading…';
      try {
        const body = new FormData();
        body.append('file', f, f.name || 'image');
        const res = await fetch('/media/upload', { method: 'POST', headers: { 'X-CSRF-Token': csrf }, credentials: 'same-origin', body });
        let data = {};
        try { data = await res.json(); } catch (_) { /* not json */ }
        if (!res.ok || !data.location) throw new Error(data.error || 'Upload failed (' + res.status + ')');
        show(data.location);
        status.textContent = '';
      } catch (e) {
        status.textContent = e.message;
      }
      file.value = '';
    });
  });
})();
