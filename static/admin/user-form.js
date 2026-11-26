// Shows the "Press card" section only while the Journalist role is ticked.
(() => {
  const box = document.getElementById('press-card');
  const toggles = [...document.querySelectorAll('input[data-journalist]')];
  if (!box || !toggles.length) return; // no role boxes on this page: keep what the server decided
  const sync = () => { box.hidden = !toggles.some((t) => t.checked); };
  toggles.forEach((t) => t.addEventListener('change', sync));
  sync();
})();
