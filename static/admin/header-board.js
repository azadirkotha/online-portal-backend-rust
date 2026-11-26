// Header highlights board: one small drop zone + the pool of published stories.
(() => {
  const form = document.getElementById('hh-form');
  if (!form || !window.Sortable) return;

  const zone = form.querySelector('.fp-list[data-zone="header"]');
  const pool = form.querySelector('.fp-list[data-zone="pool"]');
  const capacity = Number(zone.dataset.capacity || 3);
  const counter = form.querySelector('.fp-count');
  const refresh = () => { counter.textContent = zone.children.length + '/' + capacity; };

  [zone, pool].forEach((list) => {
    new Sortable(list, {
      group: 'hh',
      animation: 150,
      draggable: '.fp-item',
      ghostClass: 'fp-ghost',
      filter: '.fp-remove',
      preventOnFilter: false,
      onAdd(evt) {
        if (list === zone && zone.children.length > capacity) {
          // full: the story that was already last goes back to the pool
          const bumped = [...zone.children].reverse().find((c) => c !== evt.item);
          if (bumped) pool.prepend(bumped);
        }
        refresh();
      },
      onRemove: refresh,
      onSort: refresh,
    });
  });

  form.addEventListener('click', (e) => {
    const btn = e.target.closest('.fp-remove');
    if (!btn) return;
    pool.prepend(btn.closest('.fp-item'));
    refresh();
  });

  const search = document.getElementById('fp-search');
  if (search) search.addEventListener('input', () => {
    const q = search.value.trim().toLowerCase();
    pool.querySelectorAll('.fp-item').forEach((li) => { li.hidden = q && !li.textContent.toLowerCase().includes(q); });
  });

  form.addEventListener('submit', () => {
    document.getElementById('in-ids').value = [...zone.querySelectorAll('.fp-item')].map((li) => li.dataset.id).join(',');
  });
  refresh();
})();
