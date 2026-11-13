const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (s, r = document) => r.querySelector(s);

function h(tag, attrs = {}, ...kids) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === 'class') e.className = v;
    else if (k.startsWith('on')) e.addEventListener(k.slice(2), v);
    else if (v === true) e.setAttribute(k, '');
    else if (v !== false && v != null) e.setAttribute(k, v);
  }
  for (const k of kids.flat()) {
    if (k == null || k === false) continue;
    e.append(k.nodeType ? k : document.createTextNode(k));
  }
  return e;
}

let cfg = null;        // working copy, edited by the forms
let saved = '';        // JSON of what is on disk / applied
let boot = {};
let status = null;

/* ---------- toast + dirty tracking ---------- */
let toastTimer;
function toast(msg, bad = false) {
  const t = $('#toast');
  t.textContent = String(msg);
  t.className = 'show' + (bad ? ' bad' : '');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (t.className = ''), bad ? 6000 : 2200);
}

function markDirty() {
  const d = JSON.stringify(cfg) !== saved;
  $('#savebar').classList.toggle('dirty', d);
  $('#dirtyMsg').textContent = d ? 'Unsaved changes' : 'All changes saved';
  $('#save').disabled = !d;
  $('#revert').disabled = !d;
}

function patchSaved(fn) {
  const s = JSON.parse(saved);
  fn(s);
  saved = JSON.stringify(s);
  markDirty();
}

/* ---------- form builders ---------- */
function numInput(obj, key, { min, max, step = 1, int = true, nullable = false } = {}) {
  const inp = h('input', { type: 'number', min, max, step, value: obj[key] ?? '' });
  inp.addEventListener('input', () => {
    if (inp.value === '') {
      if (nullable) obj[key] = null;
      markDirty();
      return;
    }
    const v = Number(inp.value);
    if (Number.isNaN(v)) return;
    obj[key] = int ? Math.round(v) : v;
    markDirty();
  });
  return inp;
}

function textInput(obj, key, { nullable = false, placeholder = '' } = {}) {
  const inp = h('input', { type: 'text', value: obj[key] ?? '', placeholder, spellcheck: 'false' });
  inp.addEventListener('input', () => {
    obj[key] = nullable && inp.value.trim() === '' ? null : inp.value;
    markDirty();
  });
  return inp;
}

// Custom dropdown instead of a native <select>: on Linux (WebKitGTK) native selects are drawn by
// GTK, which ignores the page's colours and gives light-on-white text in dark mode.
function selectInput(obj, key, options) {
  const labelOf = v => (options.find(([o]) => o === v) || options[0])[1];
  const btn = h('button', { type: 'button', class: 'dd-btn', 'aria-haspopup': 'listbox', 'aria-expanded': 'false' }, labelOf(obj[key]));
  const list = h('ul', { class: 'dd-list', role: 'listbox', hidden: true });
  const wrap = h('div', { class: 'dd' }, btn, list);
  let active = -1;
  const items = options.map(([v, label]) => h('li', { class: 'dd-item', role: 'option', 'data-v': v }, label));
  items.forEach((li, i) => {
    li.addEventListener('mousedown', e => { e.preventDefault(); pick(i); });
    li.addEventListener('mousemove', () => highlight(i));
    list.append(li);
  });
  function highlight(i) {
    active = i;
    items.forEach((li, n) => li.classList.toggle('active', n === i));
  }
  function open() {
    closeDropdowns(wrap);
    list.hidden = false;
    btn.setAttribute('aria-expanded', 'true');
    highlight(Math.max(0, options.findIndex(([o]) => o === obj[key])));
  }
  function close() { list.hidden = true; btn.setAttribute('aria-expanded', 'false'); }
  function pick(i) {
    obj[key] = options[i][0];
    btn.textContent = options[i][1];
    close();
    markDirty();
  }
  btn.addEventListener('click', () => (list.hidden ? open() : close()));
  btn.addEventListener('blur', close);
  btn.addEventListener('keydown', e => {
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      if (list.hidden) { open(); return; }
      highlight((active + (e.key === 'ArrowDown' ? 1 : options.length - 1)) % options.length);
    } else if ((e.key === 'Enter' || e.key === ' ') && !list.hidden) {
      e.preventDefault();
      pick(active);
    } else if (e.key === 'Escape') close();
  });
  return wrap;
}

function closeDropdowns(except) {
  document.querySelectorAll('.dd').forEach(d => {
    if (d === except) return;
    d.querySelector('.dd-list').hidden = true;
    d.querySelector('.dd-btn').setAttribute('aria-expanded', 'false');
  });
}

function checkInput(obj, key) {
  const c = h('input', { type: 'checkbox', checked: !!obj[key] });
  c.addEventListener('change', () => { obj[key] = c.checked; markDirty(); });
  return c;
}

const KINDS = [['shock', 'Shock'], ['vibrate', 'Vibrate'], ['sound', 'Sound']];

const field = (label, input, hint) =>
  h('label', { class: 'field' }, h('span', { class: 'lbl' }, label), input, hint ? h('small', {}, hint) : null);
const checkField = (label, obj, key, hint) =>
  h('label', { class: 'field check' }, checkInput(obj, key), h('span', {}, label, hint ? h('small', {}, ' – ' + hint) : null));
const card = (title, ...kids) => h('div', { class: 'card' }, h('h3', {}, title), ...kids);
const fields = (...kids) => h('div', { class: 'fields' }, ...kids);

/* ---------- settings panels ---------- */
function renderTiers(root) {
  root.append(h('p', { class: 'hint' },
    'A hit (or several hits landing together) picks the highest tier whose minimum damage it reaches. ' +
    'DoT ticks are pooled first and use the same tiers - see Damage over time below.'));
  const rows = cfg.tiers.map((t, i) => h('tr', {},
    h('td', {}, textInput(t, 'name')),
    h('td', {}, numInput(t, 'min_damage', { min: 1 })),
    h('td', {}, selectInput(t, 'kind', KINDS)),
    h('td', {}, numInput(t, 'intensity', { min: 0, max: 100 })),
    h('td', {}, numInput(t, 'duration_ms', { min: 300, step: 50 })),
    h('td', {}, h('div', { class: 'actions' },
      h('button', { class: 'ghost', title: 'Fire this tier now', onclick: () => testShock(t.name) }, 'Test'),
      h('button', { class: 'ghost danger', title: 'Remove', onclick: () => { cfg.tiers.splice(i, 1); markDirty(); renderSettings(); } }, '✕')))));
  root.append(card('Hit tiers',
    h('table', {},
      h('thead', {}, h('tr', {}, ['Name', 'Min damage', 'Type', 'Intensity %', 'Duration ms', ''].map(x => h('th', {}, x)))),
      h('tbody', {}, rows)),
    h('p', {}, h('button', { class: 'ghost', onclick: () => {
      const last = cfg.tiers[cfg.tiers.length - 1];
      cfg.tiers.push({ name: 'tier' + (cfg.tiers.length + 1), min_damage: (last ? last.min_damage : 0) + 25, kind: 'shock', intensity: 10, duration_ms: 400 });
      markDirty(); renderSettings();
    } }, '+ Add tier'))));

  const d = cfg.death;
  root.append(card('Death',
    fields(
      field('Type', selectInput(d, 'kind', KINDS)),
      field('Intensity %', numInput(d, 'intensity', { min: 0, max: 100 })),
      field('Duration ms', numInput(d, 'duration_ms', { min: 300, step: 50 })),
      field('Debounce ms', numInput(d, 'debounce_ms', { min: 0, step: 100 }), 'the game prints the death line several times'),
      field('Quiet after death ms', numInput(d, 'suppress_ms', { min: 0, step: 100 }), 'damage is ignored this long')),
    h('p', {}, h('button', { class: 'ghost', onclick: () => testShock('death') }, 'Test death'))));
}

function renderDamage(root) {
  renderTiers(root);
  const dm = cfg.damage, dot = cfg.dot, lim = cfg.limits;
  root.append(card('Hit detection',
    fields(
      field('DoT tick max damage', numInput(dm, 'tick_max', { min: 0 }), 'hits at or below this count as DoT ticks'),
      field('Burst window ms', numInput(dm, 'burst_ms', { min: 0, step: 50 }), 'hits this close together merge into one shock'),
      field('Cooldown ms', numInput(dm, 'cooldown_ms', { min: 0, step: 50 }), 'a higher tier can still break through'))));
  root.append(card('Damage over time',
    h('p', { class: 'hint' }, 'Ticks pool up and fire as ONE shock when they stop, so a burn does not turn into a buzzing barrage.'),
    checkField('Pool DoT ticks', dot, 'enabled'),
    fields(
      field('Quiet period ms', numInput(dot, 'quiet_ms', { min: 100, step: 100 }), 'no tick for this long = flush'),
      field('Max hold ms', numInput(dot, 'max_hold_ms', { min: 500, step: 500 }), 'a DoT that never stops still flushes this often'),
      field('Scale', numInput(dot, 'scale', { min: 0, step: 0.05, int: false }), 'pooled total × scale picks the tier'))));
  root.append(card('Hard limits',
    h('p', { class: 'hint' }, 'Ceilings for everything, including death and the Test buttons. Per-target caps can only lower these.'),
    fields(
      field('Max intensity %', numInput(lim, 'max_intensity', { min: 0, max: 100 })),
      field('Max duration ms', numInput(lim, 'max_duration_ms', { min: 300, step: 100 })))));
}

function renderTargets(root) {
  root.append(h('p', { class: 'hint' },
    'Each target is an OpenShock API token plus the shockers it may control. Tokens can be scoped to specific shockers ' +
    'and given an expiry in OpenShock, which is the safest thing to hand a friend. Mark your own as "me".'));
  if (!cfg.group.shared_fate && !cfg.targets.some(t => t.enabled && t.is_me)) {
    root.append(h('div', { class: 'banner warn' },
      'Shared fate is off and no enabled target is marked "me" - nothing will fire. Mark one as "me" or turn shared fate on.'));
  }
  cfg.targets.forEach((t, i) => {
    const token = h('input', { type: 'password', value: t.token, placeholder: 'OpenShock API token', spellcheck: 'false', autocomplete: 'off' });
    token.addEventListener('input', () => { t.token = token.value.trim(); markDirty(); });
    const eye = h('button', { class: 'ghost', type: 'button', onclick: () => { token.type = token.type === 'password' ? 'text' : 'password'; } }, 'Show');
    const ta = h('textarea', { placeholder: 'one shocker ID per line', spellcheck: 'false' }, (t.shockers || []).join('\n'));
    ta.addEventListener('input', () => { t.shockers = ta.value.split(/[\s,]+/).filter(Boolean); markDirty(); });
    root.append(h('div', { class: 'card' },
      h('div', { class: 'targethead' },
        textInput(t, 'name', { placeholder: 'name' }),
        h('button', { class: 'ghost danger', onclick: () => { cfg.targets.splice(i, 1); markDirty(); renderSettings(); } }, 'Remove')),
      fields(
        checkField('Enabled', t, 'enabled'),
        checkField('This is me', t, 'is_me', 'fires even when shared fate is off')),
      h('div', { class: 'fields wide', style: 'margin-top:10px' },
        field('API token', h('div', { class: 'tokenrow' }, token, eye)),
        field('Shocker IDs', ta)),
      h('div', { class: 'fields', style: 'margin-top:10px' },
        field('Max intensity % (optional)', numInput(t, 'max_intensity', { min: 0, max: 100, nullable: true })),
        field('Max duration ms (optional)', numInput(t, 'max_duration_ms', { min: 300, step: 100, nullable: true })))));
  });
  root.append(h('button', { class: 'ghost', onclick: () => {
    cfg.targets.push({ name: 'friend' + (cfg.targets.length + 1), token: '', shockers: [], enabled: true, is_me: cfg.targets.length === 0 });
    markDirty(); renderSettings();
  } }, '+ Add target'));
}

function renderAdvanced(root) {
  const detect = h('button', { class: 'ghost', onclick: async () => {
    const d = await invoke('detect_log_dir');
    if (d) { cfg.log.dir = d; markDirty(); renderSettings(); toast('Found ' + d); }
    else toast('Could not find the VRChat log folder', true);
  } }, 'Detect');
  root.append(card('VRChat log',
    h('p', { class: 'hint' }, 'Leave the folder empty to auto-detect (Proton/Steam on Linux, AppData on Windows).'),
    fields(
      field('Log folder', h('div', { class: 'tokenrow' }, textInput(cfg.log, 'dir', { nullable: true, placeholder: 'auto-detect' }), detect)),
      checkField('Only fire during a fight', cfg.log, 'only_in_fight', 'ignore damage/death in the lobby and intermissions'))));
  root.append(card('OpenShock API',
    fields(
      field('Base URL', textInput(cfg.api, 'base_url'), 'change for a self-hosted server'),
      field('Name shown in OpenShock logs', textInput(cfg.api, 'custom_name')),
      field('Timeout ms', numInput(cfg.api, 'timeout_ms', { min: 500, step: 500 })))));
  root.append(card('Config file',
    h('p', { class: 'hint' }, 'Settings (including API tokens) are stored here, readable only by you:'),
    h('code', {}, boot.config_path || '')));
}

function renderSettings() {
  const panels = { damage: renderDamage, targets: renderTargets, advanced: renderAdvanced };
  for (const [name, fn] of Object.entries(panels)) {
    const root = $(`[data-panel="${name}"]`);
    root.replaceChildren();
    fn(root);
  }
}

async function testShock(tier) {
  if (JSON.stringify(cfg) !== saved) { toast('Save & apply first - tests use the saved settings', true); return; }
  await invoke('test_shock', { tier });
}

/* ---------- status + live feed ---------- */
function applyStatus(s) {
  status = s;
  const pill = $('#pill');
  let cls = 'info', text = 'Watching';
  if (s.error) { cls = 'bad'; text = 'Problem'; }
  else if (!s.attached) { cls = 'warn'; text = 'Waiting for VRChat log'; }
  else if (s.paused) { cls = 'warn'; text = 'Paused'; }
  else if (s.armed) { cls = 'ok'; text = 'In a fight'; }
  pill.className = 'pill ' + cls;
  pill.textContent = text;

  $('#t-shared').checked = s.shared_fate;
  $('#t-dry').checked = s.dry_run;
  $('#t-paused').checked = s.paused;

  $('#st-file').textContent = s.log_file || 'none yet';
  $('#st-fight').textContent = s.armed ? 'in a fight' : 'idle';
  $('#st-targets').textContent = `${s.active_targets} of ${s.total_targets} will fire`;
  $('#st-mode').textContent = (s.dry_run ? 'dry run' : 'live') + (s.shared_fate ? ' · shared fate' : ' · solo') + (s.paused ? ' · paused' : '');
  const err = $('#st-error');
  err.hidden = !s.error;
  err.textContent = s.error || '';
}

const feed = $('#feed');
let tickRow = null, tickCount = 0, tickSum = 0;

function addRow(cls, ...kids) {
  const li = h('li', { class: 'row ' + cls }, h('time', {}, new Date().toLocaleTimeString([], { hour12: false })), ...kids);
  feed.prepend(li);
  while (feed.children.length > 300) feed.lastChild.remove();
  return li;
}

function addTick(amount) {
  if (tickRow && tickRow === feed.firstChild) {
    tickCount++; tickSum += amount;
    tickRow.lastChild.textContent = `DoT ×${tickCount} (−${tickSum})`;
  } else {
    tickCount = 1; tickSum = amount;
    tickRow = addRow('tick', h('span', {}, `DoT ×1 (−${amount})`));
  }
}

const OUTCOME = { sent: 'sent', dry_run: 'dry run', failed: 'failed', paused: 'paused' };

listen('engine', ({ payload: ev }) => {
  switch (ev.type) {
    case 'status': applyStatus(ev); break;
    case 'hit':
      if (ev.tick) addTick(ev.amount);
      else { tickRow = null; addRow('hit', h('b', {}, '−' + ev.amount), h('span', {}, ev.source || '(no source)')); }
      break;
    case 'death': tickRow = null; addRow('death', h('span', {}, 'You died')); break;
    case 'fight': tickRow = null; addRow('fight', h('span', {}, ev.armed ? 'Fight started' : 'Fight ended')); break;
    case 'delivery':
      tickRow = null;
      addRow('delivery',
        h('span', { class: 'tag ' + ev.outcome }, OUTCOME[ev.outcome] || ev.outcome),
        h('span', {}, ev.outcome === 'paused' ? `${ev.tier} dropped (paused)`
          : `${ev.tier} → ${ev.target}: ${ev.kind} ${ev.intensity}% · ${ev.duration_ms} ms`),
        h('small', {}, [ev.reason, ev.detail].filter(Boolean).join(' · ')));
      if (ev.outcome === 'failed') toast(`Delivery to ${ev.target} failed: ${ev.detail}`, true);
      break;
    case 'notice':
      tickRow = null;
      addRow('fight', h('span', {}, ev.text));
      if (ev.level !== 'info') toast(ev.text, ev.level === 'error');
      break;
  }
});

/* ---------- wiring ---------- */
$('#tabs').addEventListener('click', e => {
  const b = e.target.closest('button[data-tab]');
  if (!b) return;
  document.querySelectorAll('#tabs button').forEach(x => x.classList.toggle('on', x === b));
  document.querySelectorAll('[data-panel]').forEach(p => (p.hidden = p.dataset.panel !== b.dataset.tab));
});

$('#clear').onclick = () => { feed.replaceChildren(); tickRow = null; };

async function flag(name, input, onOk) {
  try {
    await invoke('set_flag', { name, on: input.checked });
    onOk?.(input.checked);
  } catch (err) {
    input.checked = !input.checked;
    toast(err, true);
  }
}
$('#t-shared').onchange = e => flag('shared_fate', e.target, v => { cfg.group.shared_fate = v; patchSaved(s => (s.group.shared_fate = v)); renderSettings(); });
$('#t-dry').onchange = e => flag('dry_run', e.target, v => { cfg.dry_run = v; patchSaved(s => (s.dry_run = v)); });
$('#t-paused').onchange = e => flag('paused', e.target);

$('#save').onclick = async () => {
  try {
    await invoke('save_config', { cfg });
    cfg = await invoke('get_config');
    saved = JSON.stringify(cfg);
    renderSettings();
    markDirty();
    toast('Saved and applied');
  } catch (err) {
    toast(err, true);
  }
};

$('#revert').onclick = () => { cfg = JSON.parse(saved); renderSettings(); markDirty(); };

(async function init() {
  boot = await invoke('boot_info');
  cfg = await invoke('get_config');
  saved = JSON.stringify(cfg);
  renderSettings();
  markDirty();
  const s = await invoke('get_status');
  if (s) applyStatus(s);
  if (boot.load_error) toast('Could not read your saved config - using defaults until you save. ' + boot.load_error, true);
})();
