// Sign-in page helpers: show / hide password, Caps Lock hint, busy button.
(function () {
  var form = document.getElementById('login-form');
  var pw = document.getElementById('password');
  var toggle = document.getElementById('pw-toggle');
  var caps = document.getElementById('caps-hint');
  var btn = document.getElementById('login-btn');
  if (!form || !pw) return;

  if (toggle) {
    toggle.addEventListener('click', function () {
      var show = pw.type === 'password';
      pw.type = show ? 'text' : 'password';
      toggle.setAttribute('aria-pressed', show ? 'true' : 'false');
      toggle.setAttribute('aria-label', show ? 'Hide password' : 'Show password');
      toggle.classList.toggle('on', show);
      pw.focus();
    });
  }

  function checkCaps(e) {
    if (!caps || !e.getModifierState) return;
    caps.hidden = !e.getModifierState('CapsLock');
  }
  pw.addEventListener('keyup', checkCaps);
  pw.addEventListener('keydown', checkCaps);
  pw.addEventListener('blur', function () { if (caps) caps.hidden = true; });

  form.addEventListener('submit', function () {
    if (!btn) return;
    btn.setAttribute('aria-busy', 'true');
    btn.disabled = true;
    // the form still submits; re-enable if the user comes back with the back button
    window.addEventListener('pageshow', function () {
      btn.removeAttribute('aria-busy');
      btn.disabled = false;
    });
  });
})();