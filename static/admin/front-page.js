// Front page board (drag & drop) and category ordering. Needs Sortable (self-hosted).
(() => {
  if (!window.Sortable) return;

  // ---------- main section ----------
  const board = document.getElementById('fp-form');
  if (board) {
    const lists = [...board.querySelectorAll('.fp-list')];
    const pool = board.querySelector('.fp-list[data-zone="pool"]');

    const refreshCounters = () => {
      lists.forEach((l) => {
        if (!l.dataset.capacity) return;
        const counter = l.closest('section').querySelector('.fp-count');
        if (counter) counter.textContent = l.children.length + '/' + l.dataset.capacity;
      });
    };

    lists.forEach((list) => {
      new Sortable(list, {
        group: 'fp',
        animation: 150,
        draggable: '.fp-item',
        ghostClass: 'fp-ghost',
        filter: '.fp-remove',
        preventOnFilter: false,
        onAdd(evt) {
          const cap = Number(list.dataset.capacity || 0);
          if (cap && list.children.length > cap) {
            // zone is full: the story that was already there goes back to the pool
            const bumped = [...list.children].reverse().find((c) => c !== evt.item);
            if (bumped) pool.prepend(bumped);
          }
          refreshCounters();
        },
        onRemove: refreshCounters,
        onSort: refreshCounters,
      });
    });

    board.addEventListener('click', (e) => {
      const btn = e.target.closest('.fp-remove');
      if (!btn) return;
      pool.prepend(btn.closest('.fp-item'));
      refreshCounters();
    });

    const search = document.getElementById('fp-search');
    if (search) search.addEventListener('input', () => {
      const q = search.value.trim().toLowerCase();
      pool.querySelectorAll('.fp-item').forEach((li) => {
        li.hidden = q && !li.textContent.toLowerCase().includes(q);
      });
    });

    board.addEventListener('submit', () => {
      const idsOf = (zone) => [...board.querySelectorAll('.fp-list[data-zone="' + zone + '"] .fp-item')]
        .map((li) => li.dataset.id).join(',');
      document.getElementById('in-lead').value = idsOf('lead');
      document.getElementById('in-center').value = idsOf('center');
      document.getElementById('in-left').value = idsOf('left');
    });
    refreshCounters();
  }

  // ---------- category order ----------
  const catForm = document.getElementById('cat-form');
  if (catForm) {
    const list = document.getElementById('cat-list');
    new Sortable(list, { animation: 150, handle: '.fp-handle', draggable: '.fp-item', ghostClass: 'fp-ghost' });
    catForm.addEventListener('submit', () => {
      document.getElementById('in-order').value = [...list.querySelectorAll('.fp-item')].map((li) => li.dataset.id).join(',');
    });
  }
})();
