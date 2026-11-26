// Menu builder: nested drag & drop (2 levels), delete confirmation, unsaved-changes hint.
(() => {
  const root = document.getElementById('menu-root');
  const form = document.getElementById('menu-form');
  if (!root || !form) return;

  let dirty = false;
  const dirtyHint = document.getElementById('menu-dirty');
  const markDirty = () => { dirty = true; if (dirtyHint) dirtyHint.hidden = false; };

  if (window.Sortable && root.querySelector('.fp-handle, .menu-node') && document.querySelector('.fp-handle')) {
    const lists = [root, ...root.querySelectorAll('.menu-children')];
    lists.forEach((list) => {
      const isRoot = list === root;
      new Sortable(list, {
        group: {
          name: 'menu',
          // Keep the tree at two levels: a dropdown list only accepts items that have no
          // dropdown of their own, and an item that is itself inside a dropdown cannot receive any.
          put: (to, _from, dragEl) => {
            if (isRoot) return true;
            const owner = to.el.closest('.menu-node');
            const ownerIsChild = owner && owner.parentElement.classList.contains('menu-children');
            return !ownerIsChild && !dragEl.querySelector('.menu-node');
          },
        },
        draggable: '.menu-node',
        handle: '.fp-handle',
        animation: 150,
        ghostClass: 'fp-ghost',
        fallbackOnBody: true,
        swapThreshold: 0.65,
        onStart: () => document.body.classList.add('is-dragging'),
        onEnd: () => document.body.classList.remove('is-dragging'),
        onSort: markDirty,
        onAdd: markDirty,
      });
    });
  }

  form.addEventListener('submit', () => {
    const tree = [...root.querySelectorAll(':scope > .menu-node')].map((li) => ({
      id: Number(li.dataset.id),
      children: [...li.querySelectorAll(':scope > .menu-children > .menu-node')].map((c) => Number(c.dataset.id)),
    }));
    document.getElementById('menu-tree').value = JSON.stringify(tree);
    dirty = false;
  });

  window.addEventListener('beforeunload', (e) => { if (dirty) { e.preventDefault(); e.returnValue = ''; } });

  // "Remove" buttons live inside the order form, so they post through a separate hidden form.
  document.addEventListener('click', (e) => {
    const btn = e.target.closest('[data-delete]');
    if (!btn) return;
    const li = btn.closest('.menu-node');
    const hasDropdown = li && li.querySelector(':scope > .menu-children > .menu-node');
    const msg = 'Remove "' + btn.dataset.name + '" from this menu?' + (hasDropdown ? ' Its dropdown items will be removed too.' : '');
    if (!window.confirm(msg)) return;
    const del = document.getElementById('menu-delete-form');
    del.action = '/menus/items/' + btn.dataset.delete + '/delete';
    dirty = false;
    del.submit();
  });
})();
