// Article editor page. Self-hosted (the admin CSP only allows scripts from 'self').
(() => {
  const form = document.getElementById('article-form');
  if (!form) return;
  const csrf = form.dataset.csrf;

  // ---- uploads (used by the featured photo and by the rich-text editor) ----
  async function uploadImage(file) {
    const body = new FormData();
    body.append('file', file, file.name || 'image');
    const res = await fetch('/media/upload', {
      method: 'POST',
      headers: { 'X-CSRF-Token': csrf },
      credentials: 'same-origin',
      body,
    });
    let data = {};
    try { data = await res.json(); } catch (_) { /* not json */ }
    if (!res.ok || !data.location) throw new Error(data.error || 'Upload failed (' + res.status + ')');
    return data.location;
  }

  // ---- featured photo ----
  const pickBtn = document.getElementById('featured-pick');
  const removeBtn = document.getElementById('featured-remove');
  const fileInput = document.getElementById('featured-file');
  const hidden = document.getElementById('featured-image');
  const preview = document.getElementById('featured-preview');
  const img = document.getElementById('featured-img');
  const statusEl = document.getElementById('featured-status');

  const showFeatured = (url) => {
    hidden.value = url || '';
    if (url) { img.src = url; preview.hidden = false; removeBtn.hidden = false; }
    else { img.removeAttribute('src'); preview.hidden = true; removeBtn.hidden = true; }
  };
  if (pickBtn) pickBtn.addEventListener('click', () => fileInput.click());
  if (removeBtn) removeBtn.addEventListener('click', () => showFeatured(''));
  if (fileInput) fileInput.addEventListener('change', async () => {
    const f = fileInput.files[0];
    if (!f) return;
    statusEl.textContent = 'Uploading…';
    try { showFeatured(await uploadImage(f)); statusEl.textContent = ''; }
    catch (e) { statusEl.textContent = e.message; }
    fileInput.value = '';
  });

  // ---- sub categories only show for ticked categories ----
  const catBoxes = [...form.querySelectorAll('input[data-category]')];
  const groups = [...form.querySelectorAll('.sub-group')];
  const subCard = document.getElementById('sub-card');
  const syncSubs = () => {
    const on = new Set(catBoxes.filter((b) => b.checked).map((b) => b.dataset.category));
    let any = false;
    groups.forEach((g) => { const show = on.has(g.dataset.parent); g.hidden = !show; any = any || show; });
    if (subCard) subCard.hidden = !any;
  };
  catBoxes.forEach((b) => b.addEventListener('change', syncSubs));
  syncSubs();

  // ---- tag filter ----
  const tagFilter = document.getElementById('tag-filter');
  if (tagFilter) tagFilter.addEventListener('input', () => {
    const q = tagFilter.value.trim().toLowerCase();
    document.querySelectorAll('#tag-list .choice').forEach((el) => {
      el.hidden = q && !(el.dataset.name || '').toLowerCase().includes(q);
    });
  });

  // ---- rich-text editor ----
  if (!window.tinymce) {
    console.warn('TinyMCE did not load; falling back to the plain text box.');
    return;
  }
  const fonts = [
    'Noto Sans Bengali=Noto Sans Bengali,sans-serif',
    'Noto Serif Bengali=Noto Serif Bengali,serif',
    'Hind Siliguri=Hind Siliguri,sans-serif',
    'Kalpurush=Kalpurush,Noto Sans Bengali,sans-serif',
    'SolaimanLipi=SolaimanLipi,Noto Sans Bengali,sans-serif',
    'Nikosh=Nikosh,Noto Sans Bengali,sans-serif',
    'Arial=arial,helvetica,sans-serif',
    'Georgia=georgia,serif',
    'Times New Roman=times new roman,times,serif',
    'Courier New=courier new,courier,monospace',
  ].join(';');

  tinymce.init({
    selector: '#body',
    license_key: 'gpl',
    promotion: false,
    branding: false,
    height: 640,
    min_height: 420,
    resize: true,
    menubar: 'edit insert format table',
    plugins: 'advlist autolink lists link image charmap searchreplace visualblocks code fullscreen table wordcount',
    toolbar: [
      'undo redo | blocks fontfamily fontsize | bold italic underline strikethrough | forecolor backcolor | removeformat',
      'alignleft aligncenter alignright alignjustify | bullist numlist outdent indent | blockquote link image table charmap | searchreplace visualblocks code fullscreen',
    ],
    toolbar_mode: 'wrap',
    block_formats: 'Paragraph=p; Heading 2=h2; Heading 3=h3; Heading 4=h4; Quote=blockquote',
    font_family_formats: fonts,
    font_size_formats: '12px 14px 16px 18px 20px 24px 28px 32px 40px 48px',
    content_css: ['/static/vendor/tinymce/skins/content/default/content.min.css', '/static/article-content.css'],
    // photos: caption (figcaption), alt text and title are part of the Image dialog
    image_caption: true,
    image_description: true,
    image_title: true,
    image_advtab: false,
    image_dimensions: true,
    automatic_uploads: true,
    paste_data_images: true,
    images_upload_handler: (blobInfo) => uploadImage(blobInfo.blob()),
    file_picker_types: 'image',
    file_picker_callback: (callback, _value, meta) => {
      if (meta.filetype !== 'image') return;
      const input = document.createElement('input');
      input.type = 'file';
      input.accept = 'image/jpeg,image/png,image/gif,image/webp';
      input.addEventListener('change', async () => {
        const f = input.files[0];
        if (!f) return;
        try { callback(await uploadImage(f), { alt: '' }); }
        catch (e) { window.alert(e.message); }
      });
      input.click();
    },
    relative_urls: false,
    convert_urls: false,
    link_default_target: '_blank',
    link_assume_external_targets: 'https',
    contextmenu: false,
    browser_spellcheck: false,
  });

  // keep the textarea in sync before any submit button posts the form
  form.addEventListener('submit', () => { if (window.tinymce) tinymce.triggerSave(); });
})();
