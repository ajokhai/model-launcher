// Model Launcher's window. Everything goes through the local API (the same one other apps use).
const API = 'http://127.0.0.1:11500';
const tauri = window.__TAURI__ && window.__TAURI__.core;
const $ = (id) => document.getElementById(id);
const esc = (s) => String(s ?? '').replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
const gb = (n) => `${(n || 0).toFixed(n >= 10 ? 0 : 1)} GB`;

let state = { status: null, library: [], catalog: [], filter: 'all', loading: null };

async function api(path, opts = {}) {
  const init = { method: opts.method || (opts.body ? 'POST' : 'GET'), headers: {} };
  if (opts.body instanceof FormData) init.body = opts.body;
  else if (opts.body !== undefined) { init.body = JSON.stringify(opts.body); init.headers['content-type'] = 'application/json'; }
  const r = await fetch(API + path, init);
  if (!r.ok) {
    let msg = `${r.status}`;
    try { msg = (await r.json()).error.message; } catch {}
    throw Object.assign(new Error(msg), { status: r.status });
  }
  return opts.raw ? r : r.json();
}

function toast(msg, err) {
  const t = $('toast');
  t.textContent = msg;
  t.className = 'toast show' + (err ? ' err' : '');
  clearTimeout(toast.t);
  toast.t = setTimeout(() => (t.className = 'toast'), err ? 6000 : 3000);
}

function confirmDelete(title, text, yes = 'Delete', danger = true) {
  return new Promise((resolve) => {
    $('modal-title').textContent = title;
    $('modal-text').textContent = text;
    $('modal-yes').textContent = yes;
    $('modal-yes').className = `btn ${danger ? 'danger' : 'primary'}`;
    $('modal').hidden = false;
    const done = (v) => { $('modal').hidden = true; resolve(v); };
    $('modal-yes').onclick = () => done(true);
    $('modal-no').onclick = () => done(false);
  });
}

const FIT = { great: ['great', 'Runs great'], ok: ['ok', 'Fits'], over_budget: ['over', 'Over budget'], too_big: ['too', 'Too big'] };
const fitBadge = (f) => { const [c, t] = FIT[f] || ['', f]; return `<span class="badge ${c}">${t}</span>`; };
const KIND = { chat: ['chat', 'Chat'], transcribe: ['voice', 'Speech to text'], speak: ['voice', 'Text to speech'], image: ['image', 'Image'], video: ['image', 'Video'] };
const ENGINE_FOR = { chat: 'llama_server', transcribe: 'whisper_cli', speak: 'llama_tts', image: 'sd_cli', video: 'sd_cli' };
// What goes in and what comes out, and whether that makes the model multimodal.
function modality(m) {
  const io = { chat: m.vision ? ['Text + image', 'Text'] : ['Text', 'Text'], transcribe: ['Audio', 'Text'], speak: ['Text', 'Audio'], image: ['Text', 'Image'], video: ['Text', 'Video'] }[m.kind];
  const multi = m.kind !== 'chat' || m.vision;
  return `<span class="mode ${multi ? 'multi' : 'uni'}" title="${multi ? 'Works across kinds of data' : 'One kind of data in and out'}">${multi ? 'Multimodal' : 'Unimodal'} · ${io[0]} → ${io[1]}</span>`;
}
const hf = (repo) => `<a href="#" data-link="https://huggingface.co/${esc(repo)}">${esc(repo)}</a>`;
function source(m) {
  const a = m.attribution;
  if (!a) return `<div class="source"><span>Added by you</span><span class="sep">·</span><span class="trunc">${esc(m.dir.replace(/^\/Users\/[^/]+/, '~'))}</span></div>`;
  const license = a.license_url ? `<a href="#" data-link="${esc(a.license_url)}">${esc(a.license)}</a>` : esc(a.license);
  const files = a.download_repo !== a.origin ? `<a href="#" title="Files: huggingface.co/${esc(a.download_repo)}" data-link="https://huggingface.co/${esc(a.download_repo)}">files</a><span class="sep">·</span>` : '';
  if (a.imported_from) {
    const src = a.imported_from.includes('huggingface.co/') ? hf(a.origin) : `<a href="#" data-link="${esc(a.imported_from)}">${esc(a.origin)}</a>`;
    return `<div class="source"><span>Added by you from</span><span class="sep"></span><span class="trunc">${src}</span><span class="sep">·</span><a href="#" data-link="${esc(a.imported_from)}">license on model page</a></div>`;
  }
  return `<div class="source"><span class="trunc">${a.origin ? hf(a.origin) : hf(a.download_repo)}</span><span class="sep">·</span>${files}${license}</div>`;
}
// A soft tint per maker, so a grid of cards reads at a glance without one color everywhere.
const MAKER_TINT = { Qwen: 'violet', Google: 'blue', Meta: 'indigo', OpenAI: 'sage', 'Stability AI': 'plum', OuteAI: 'green' };
const tint = (m) => MAKER_TINT[m.maker] || 'gray';
// The maker's own logo (their Hugging Face avatar, bundled in ui/logos), else a tinted monogram.
const LOGOS = new Set(['Qwen', 'google', 'meta-llama', 'openai', 'stabilityai', 'OuteAI', 'Wan-AI', 'OpenGVLab']);
function logo(m) {
  const org = (m.attribution?.origin || '').split('/')[0];
  if (LOGOS.has(org)) return `<div class="icon logo-img"><img src="logos/${org}.png" alt="${esc(m.maker)} logo"></div>`;
  return `<div class="icon t-${tint(m)}">${esc(initials(m.name))}</div>`;
}
const initials = (name) => name.replace(/[^A-Za-z0-9 ]/g, ' ').trim().split(/\s+/).slice(0, 2).map((w) => w[0]).join('').toUpperCase();

function engineReady(kind) {
  const e = state.status?.engines?.find((x) => x.engine === ENGINE_FOR[kind]);
  return !!(e && e.path);
}

async function ensureEngine(kind) {
  if (engineReady(kind)) return;
  toast('Installing the engine for this model…');
  await api('/api/engines/install', { body: { engine: ENGINE_FOR[kind] } });
  await refresh();
}

// ---------- tabs ----------
$('tabs').onclick = (e) => {
  const tab = e.target.dataset.tab;
  if (!tab) return;
  document.querySelectorAll('#tabs button').forEach((b) => b.classList.toggle('on', b === e.target));
  document.querySelectorAll('.page').forEach((p) => p.classList.toggle('on', p.id === 'page-' + tab));
  if (tab === 'connect') renderConnect();
  state.tab = tab;
  if (tab === 'bench') refreshBench();
};
$('filters').onclick = (e) => {
  if (!e.target.dataset.kind) return;
  state.filter = e.target.dataset.kind;
  document.querySelectorAll('#filters button').forEach((b) => b.classList.toggle('on', b === e.target));
  renderCatalog();
};

// ---------- data ----------
async function refresh() {
  try {
    const [status, library] = await Promise.all([api('/api/status'), api('/api/library')]);
    const downloadsChanged = JSON.stringify(Object.keys(status.downloads || {}).map((k) => [k, status.downloads[k].finished])) !==
      JSON.stringify(Object.keys(state.status?.downloads || {}).map((k) => [k, state.status.downloads[k].finished]));
    state.status = status;
    state.library = library;
    if (!state.catalog.length || downloadsChanged) state.catalog = await api('/api/catalog');
    render();
    if (state.tab === 'bench') refreshBench();
  } catch (e) {
    $('banner').innerHTML = `<div class="card"><span class="live loading"></span><div>Starting Model Launcher…</div></div>`;
  }
}

function render() {
  renderMeter();
  renderBanner();
  renderLibrary();
  renderCatalog();
  renderStudioSelects();
  renderSettings();
}

function renderMeter() {
  const s = state.status;
  const budget = s.settings.memory_budget_gb;
  const frac = s.used_gb / budget;
  $('meter-fill').style.width = Math.min(100, frac * 100) + '%';
  $('meter-fill').className = frac > 1 ? 'over' : frac > 0.85 ? 'warn' : '';
  $('meter-text').textContent = `RAM ${gb(s.used_gb)} / ${gb(budget)} · Disk ${gb(s.system.disk_free_gb)} free`;
}

function renderBanner() {
  const rs = state.status.running || [], j = state.status.job, max = state.status.max_loaded;
  let html = '';
  const u = state.status.update;
  if (u?.ready) html += `<div class="card"><span class="live"></span><div style="flex:1">Model Launcher ${esc(u.latest)} is ready. It installs by itself once nothing is running.</div><button class="btn primary sm" onclick="installUpdate()">Install now</button></div>`;
  if (rs.length) {
    html += `<div class="card dark loaded"><div class="loaded-head"><span class="live"></span><b>${rs.length} of ${max} loaded</b><span class="muted">Models stay loaded while they fit your RAM budget; the least recently used makes room.</span></div>
      ${rs.map((r) => `<div class="loaded-row"><span class="live ${r.ready ? '' : 'loading'}"></span>
        <div style="flex:1"><b>${esc(r.name)}</b> <span class="muted">${r.ready ? '' : 'loading… · '}${gb(r.memory_gb)} · ${r.vision ? 'reads images · ' : ''}${r.forced ? 'over budget, run anyway · ' : ''}idle ${Math.floor(r.idle_secs / 60)} min</span></div>
        <button class="btn primary sm" onclick="openChat('${esc(r.id)}')" ${r.ready ? '' : 'disabled'}>Open chat</button>
        <button class="btn icon-only" onclick="unload('${esc(r.id)}')">Unload</button></div>`).join('')}</div>`;
  }
  if (j) html += `<div class="card"><span class="live loading"></span><div>${esc(j.label)}… using ${gb(j.memory_gb)}</div></div>`;
  $('banner').innerHTML = html;
}

function modelCard(m) {
  const running = (state.status.running || []).some((r) => r.id === m.id);
  const [cls, label] = KIND[m.kind];
  const main = m.kind === 'chat'
    ? `<button class="btn primary" onclick="openChat('${esc(m.id)}')" ${state.loading === m.id ? 'disabled' : ''}>${state.loading === m.id ? 'Loading…' : running ? 'Open chat' : 'Chat'}</button>`
    : `<button class="btn primary" onclick="tryStudio()">Try in Studio</button>`;
  return `<div class="card model ${running ? 'dark' : ''}">
    <div class="model-head">${logo(m)}
      <div class="title"><h3>${esc(m.name)}</h3><div class="muted">${esc(m.maker || 'Added by you')} · ${label}</div></div>${fitBadge(m.fit)}</div>
    <div class="pills">${modality(m)}<span class="pill">${gb(m.size_gb)} · needs ~${gb(m.need_gb)}</span></div>
    <div class="actions">${main}
      ${running ? `<button class="btn icon-only" onclick="unload('${esc(m.id)}')">Unload</button>` : ''}
      <span class="spacer"></span>
      <button class="btn icon-only" title="Show in Finder" onclick="reveal('${esc(m.id)}')">Folder</button>
      <button class="btn icon-only" title="Delete" onclick="removeModel('${esc(m.id)}')">Delete</button></div>
    ${source(m)}
  </div>`;
}

function renderPending() {
  const dl = state.status?.downloads || {};
  const names = new Map(state.catalog.map((m) => [m.id, m.name]));
  const rows = Object.entries(dl).filter(([, d]) => !d.finished || d.error);
  $('pending').innerHTML = rows.map(([id, d]) => `<div class="card pending-row"><div style="flex:1"><b>Downloading ${esc(names.get(id) || id)}</b>
    ${d.error ? `<div class="muted" style="color:var(--red)">${esc(d.error)}</div>` : `<div class="progress"><i style="width:${d.total_gb ? (100 * d.done_gb / d.total_gb).toFixed(1) : 2}%"></i></div><div class="muted">${gb(d.done_gb)} of ${d.total_gb > .01 ? gb(d.total_gb) : '…'}</div>`}</div>
    ${d.error ? '' : `<button class="btn icon-only" onclick="cancelDl('${esc(id)}')">Cancel</button>`}</div>`).join('');
}

function renderLibrary() {
  renderPending();
  const lib = state.library;
  if (!lib.length) {
    $('library').innerHTML = `<div class="empty"><h3>No models yet</h3><p>Get one from Discover. Look for <span class="badge great">Runs great</span>.</p><br><button class="btn primary" onclick="document.querySelector('[data-tab=discover]').click()">Discover models</button> <button class="btn" onclick="openAdd()">Add your own model</button></div>`;
    return;
  }
  const groups = [['Chat', (m) => m.kind === 'chat'], ['Voice', (m) => m.kind === 'transcribe' || m.kind === 'speak'], ['Image', (m) => m.kind === 'image'], ['Video', (m) => m.kind === 'video']];
  $('library').innerHTML = groups
    .map(([name, f]) => { const ms = lib.filter(f); return ms.length ? `<h2 class="section">${name}</h2><div class="grid">${ms.map(modelCard).join('')}</div>` : ''; })
    .join('') + addCard();
}

/** The way in for models that aren't in Discover, at the end of Library and Discover. */
function addCard() {
  return `<div class="add-tile"><div><b>Add your own model</b><div class="muted">From a Hugging Face link, files already on this Mac, or a folder you make yourself. Any GGUF chat model, Whisper or Stable Diffusion file works.</div></div><button class="btn primary" onclick="openAdd()">+ Add a model</button></div>`;
}
function openAdd() { $('add').hidden = false; }

function renderCatalog() {
  const f = state.filter;
  const dl = state.status?.downloads || {};
  const list = state.catalog.filter((m) => f === 'all' || (f === 'chat' && m.kind === 'chat') || (f === 'voice' && (m.kind === 'transcribe' || m.kind === 'speak')) || (f === 'image' && m.kind === 'image') || (f === 'video' && m.kind === 'video'));
  $('catalog').innerHTML = list.map((m) => {
    const [cls, label] = KIND[m.kind];
    const d = dl[m.id];
    let action;
    if (m.installed) action = `<span class="pill done">✓ Installed</span>`;
    else if (d && !d.finished && !d.error) action = `<div style="flex:1"><div class="progress"><i style="width:${(100 * d.done_gb / d.total_gb).toFixed(1)}%"></i></div><div class="muted">${gb(d.done_gb)} of ${gb(d.total_gb)}</div></div><button class="btn icon-only" onclick="cancelDl('${m.id}')">Cancel</button>`;
    else action = `<button class="btn primary sm" onclick="download('${m.id}')" ${m.fit === 'too_big' ? 'disabled' : ''}>Download · ${gb(m.size_gb)}</button>${d?.error ? `<span class="muted">${esc(d.error)}</span>` : ''}`;
    return `<div class="card model">
      <div class="model-head">${logo(m)}<div class="title"><h3>${esc(m.name)}</h3><div class="muted">${esc(m.maker)} · ${label}</div></div>${fitBadge(m.fit)}</div>
      <p class="blurb">${esc(m.blurb)}</p>
      <div class="pills">${modality(m)}<span class="pill">needs ~${gb(m.need_gb)}</span></div>
      <div class="actions">${action}</div>
      ${source(m)}</div>`;
  }).join('');
}

// ---------- actions ----------
async function openChat(id) {
  try {
    const m = state.library.find((x) => x.id === id);
    await ensureEngine('chat');
    state.loading = id;
    render();
    let r;
    try {
      r = await api('/api/launch', { body: { id } });
    } catch (e) {
      if (e.status !== 507) throw e;
      // not enough RAM: offer to run it anyway
      state.loading = null;
      render();
      const go = await confirmDelete('Run it anyway?', `${e.message.replace(/ (Raise|Close).*$/, '')}\n\nRunning anyway unloads your other chat models and puts what doesn't fit on the GPU onto the CPU. It will work, but it can be slow, and other apps may slow down while it runs. Unload it when you're done.`, 'Run anyway', false);
      if (!go) return refresh();
      state.loading = id;
      render();
      r = await api('/api/launch', { body: { id, force: true } });
    }
    state.loading = null;
    if (tauri) await tauri.invoke('open_chat', { url: r.url, title: r.name });
    else window.open(r.url);
    refresh();
  } catch (e) {
    state.loading = null;
    toast(e.message, true);
    refresh();
  }
}
async function unload(id) { await api('/api/stop', { body: id ? { id } : {} }); refresh(); }
async function reveal(id) { await api('/api/reveal', { body: id ? { id } : {} }); }
async function removeModel(id) {
  const m = state.library.find((x) => x.id === id);
  if (!(await confirmDelete(`Delete ${m.name}?`, `This removes its folder from ~/Models and frees ${gb(m.size_gb)}. You can download it again later.`))) return;
  try { const r = await api('/api/delete', { body: { id } }); toast(`Deleted. ${gb(r.freed_gb)} freed.`); } catch (e) { toast(e.message, true); }
  state.catalog = await api('/api/catalog');
  refresh();
}
async function download(id) {
  try {
    const m = state.catalog.find((x) => x.id === id);
    await api('/api/download', { body: { id } });
    refresh();
    ensureEngine(m.kind).catch((e) => toast(e.message, true));
  } catch (e) { toast(e.message, true); }
}
async function cancelDl(id) { await api('/api/download/cancel', { body: { id } }); refresh(); }
// ---------- benchmark ----------
async function refreshBench() {
  try { state.bench = await api('/api/bench'); } catch { return; }
  renderBench();
}
function renderBench() {
  const b = state.bench;
  if (!b) return;
  const cur = b.current;
  const chats = state.library.filter((m) => m.kind === 'chat');
  if (!b.available) {
    $('bench-setup').innerHTML = `<div class="card"><div class="card-title">Install Computer Use first</div><p class="hint">The benchmark uses Computer Use to see the screen and click. Download it, drag it to Applications, open it once, and in its <b>Settings</b> allow <b>Accessibility</b> and <b>Screen Recording</b>.</p><br><button class="btn primary" data-link="https://github.com/ajokhai/computer-use-lite/releases">Get Computer Use</button></div>`;
  } else if (!chats.length) {
    $('bench-setup').innerHTML = `<div class="card"><div class="card-title">Get a chat model first</div><p class="hint">Download one in Discover. Models that read images (Qwen3-VL, Gemma 3, InternVL3) are the interesting ones here.</p></div>`;
  } else if (!cur || !cur.running) {
    const keep = $('bench-model')?.value, keepTask = $('bench-task')?.value || state.benchTask || 'tweet';
    const tasks = b.tasks || [];
    const t = tasks.find((x) => x.id === keepTask) || tasks[0];
    $('bench-setup').innerHTML = `<div class="card bench-start">
      <div class="bench-pick"><label class="field"><span>Benchmark</span><select id="bench-task" onchange="state.benchTask = this.value; renderBench()">${tasks.map((x) => `<option value="${esc(x.id)}">${esc(x.name)}${x.builtin ? ' (built in)' : ''}</option>`).join('')}</select></label>
      <label class="field"><span>Model</span><select id="bench-model">${chats.map((m) => `<option value="${esc(m.id)}">${esc(m.name)} · ${gb(m.size_gb)}${m.vision ? ' · reads images' : ''}</option>`).join('')}</select></label></div>
      <div class="bench-task"><div class="muted">${esc(t.prompt.replace(/\{model\}/g, 'the model'))}</div><div class="row"><small class="note">${t.minutes} minutes · you check: ${esc(t.check)}</small><span class="spacer"></span>${t.builtin ? '' : `<button class="btn ghost sm" onclick="editTask('${esc(t.id)}')">Edit</button>`}</div></div>
      <ul class="bench-notes">${t.id === 'tweet' ? '<li>Be signed in to X in Chrome. The test posts a <b>real tweet</b> from that account.</li>' : ''}
      <li>It drives this Mac for up to ${t.minutes} minutes, so leave the mouse and keyboard alone.${b.displays > 1 ? ' It stays on your built-in display; your other screen is left alone.' : ''}</li>
      <li>Stop it any time: press <b>Escape three times</b>, or move the mouse hard into a screen corner.</li></ul>
      <div class="row"><button class="btn primary" onclick="startBench()">Run the test</button><span class="spacer"></span><button class="btn ghost" onclick="api('/api/bench/reveal', { body: {} })">Show reports in Finder</button></div></div>`;
    if (keep && chats.some((m) => m.id === keep)) $('bench-model').value = keep;
    $('bench-task').value = t.id;
  } else {
    $('bench-setup').innerHTML = '';
  }
  if (cur) {
    const done = !cur.running;
    const ask = done && !cur.verdict;
    $('bench-run').innerHTML = `<div class="card ${cur.running ? 'dark' : ''}">
      <div class="row"><span class="live ${cur.running ? 'loading' : ''}"></span><b>${esc(cur.model_name)}</b><span class="muted">${esc(cur.task_name)}</span><span class="muted">${cur.running ? `running · ${cur.actions} tool calls` : `${cur.seconds} s · ${cur.actions} tool calls`}</span><span class="spacer"></span>${cur.running ? '<button class="btn" onclick="api(\'/api/bench/stop\', { body: {} })">Stop</button>' : ''}</div>
      <pre class="bench-log">${esc(cur.log.slice(-14).join('\n') || 'Starting…')}</pre>
      ${done ? `<p class="hint">${esc(cur.result)}</p>` : ''}
      ${ask ? `<div class="bench-ask"><b>${esc(cur.check)}</b><div class="row"><button class="btn primary" onclick="benchVerdict('pass')">Yes, it worked</button><button class="btn" onclick="benchVerdict('fail')">No</button>${cur.task_id === 'tweet' ? '<button class="btn ghost" data-link="https://x.com">Open X</button>' : ''}</div><small class="note">A model saying "done" isn't a pass. Only what you can see counts.</small></div>` : ''}
      ${cur.verdict ? `<div class="bench-ask"><b>${cur.verdict === 'pass' ? 'Passed ✅' : 'Failed ❌'}. Saved to your reports.</b><div class="row"><button class="btn primary" onclick="shareBench('x')">Share on X</button><button class="btn" onclick="shareBench('issue')">Open an issue with the report</button><button class="btn ghost" onclick="copyBench()">Copy report</button></div></div>` : ''}
    </div>`;
  } else {
    $('bench-run').innerHTML = '';
  }
  const rows = b.history || [];
  $('bench-history').innerHTML = rows.length ? `<div class="card"><table class="bench-table"><tr><th>When</th><th>Benchmark</th><th>Model</th><th>Mac</th><th>Result</th><th>Time</th><th>Calls</th></tr>${rows.slice(0, 30).map((r) => `<tr><td>${esc((r.at || '').slice(0, 16))}</td><td>${esc(r.task_name || (r.task === 'tweet' ? 'Post a tweet' : r.task))}</td><td>${esc(r.model_name || r.model)}</td><td>${r.machine ? `${r.machine.ram_gb} GB` : ''}</td><td>${r.verdict === 'pass' ? '<span class="badge great">Pass</span>' : r.verdict === 'fail' ? '<span class="badge too">Fail</span>' : '<span class="badge">Not checked</span>'}</td><td>${r.seconds} s</td><td>${r.actions}</td></tr>`).join('')}</table></div>` : '<p class="hint">No runs yet. Results from Computer Use\'s <code>scripts/benchmark.py</code> show up here too.</p>';
}
async function startBench() {
  const id = $('bench-model').value, task = $('bench-task').value;
  try {
    await ensureEngine('chat');
    toast('Loading the model…');
    try { await api('/api/bench/start', { body: { id, task } }); } catch (e) {
      if (e.status !== 507) throw e;
      if (!(await confirmDelete('Run it anyway?', `${e.message.replace(/ (Raise|Close).*$/, '')}\n\nRunning anyway unloads your other chat models and puts what doesn't fit on the GPU onto the CPU. It works, but slower.`, 'Run anyway', false))) return;
      await api('/api/bench/start', { body: { id, task, force: true } });
    }
    refreshBench();
  } catch (e) { toast(e.message, true); }
}
function editTask(id) {
  const t = id ? (state.bench?.tasks || []).find((x) => x.id === id) : null;
  state.editing = id || '';
  $('task-title').textContent = t ? 'Edit benchmark' : 'New benchmark';
  $('t-name').value = t?.name || '';
  $('t-prompt').value = t?.prompt || '';
  $('t-check').value = t?.check || '';
  $('t-minutes').value = String(t?.minutes || 10);
  $('t-delete').hidden = !t;
  $('t-delete').onclick = async () => {
    if (!(await confirmDelete(`Delete "${t.name}"?`, 'Its past runs stay in your reports.'))) return;
    await api('/api/bench/tasks/delete', { body: { id: t.id } });
    $('task-modal').hidden = true; state.benchTask = 'tweet'; refreshBench();
  };
  $('task-modal').hidden = false;
  $('t-name').focus();
}
async function saveTask() {
  try {
    const r = await api('/api/bench/tasks', { body: { id: state.editing, name: $('t-name').value, prompt: $('t-prompt').value, check: $('t-check').value, minutes: +$('t-minutes').value } });
    $('task-modal').hidden = true; state.benchTask = r.id; toast('Benchmark saved'); refreshBench();
  } catch (e) { toast(e.message, true); }
}
async function benchVerdict(verdict) {
  try { state.benchReport = (await api('/api/bench/verdict', { body: { verdict } })).report; refreshBench(); } catch (e) { toast(e.message, true); }
}
function benchSummary() {
  const c = state.bench.current, ram = Math.round(state.status.system.ram_total_gb);
  return { c, ram, verdict: c.verdict === 'pass' ? 'passed' : 'failed' };
}
function shareBench(where) {
  const { c, ram, verdict } = benchSummary();
  const repo = 'https://github.com/ajokhai/computer-use-lite';
  const url = where === 'x'
    ? 'https://x.com/intent/post?' + new URLSearchParams({ text: `${c.model_name} ${verdict} the "${c.task_name}" computer-use test on my ${ram} GB Mac: ${c.seconds} s, ${c.actions} tool calls. Tested with Model Launcher. ${repo}` })
    : `${repo}/issues/new?` + new URLSearchParams({ title: `Benchmark: ${c.model_name} ${verdict} "${c.task_name}" (${ram} GB Mac)`, body: (state.benchReport || '').slice(0, 6000) });
  if (tauri) tauri.invoke('open_url', { url }); else window.open(url);
}
async function copyBench() {
  try { await navigator.clipboard.writeText(state.benchReport || ''); toast('Report copied'); } catch { toast("Couldn't copy; the report is in Finder", true); }
}

function tryStudio() { document.querySelector('[data-tab=studio]').click(); }
$('open-models').onclick = (e) => { e.preventDefault(); reveal(); };
document.addEventListener('click', (e) => {
  const link = e.target.closest('[data-link]');
  if (link) { e.preventDefault(); tauri ? tauri.invoke('open_url', { url: link.dataset.link }) : window.open(link.dataset.link); }
  const copy = e.target.closest('[data-copy]');
  if (copy) { navigator.clipboard.writeText($(copy.dataset.copy).textContent); toast('Copied'); }
});

// ---------- add a model ----------
let addWay = 'link', picked = [];
$('add-open').onclick = openAdd;
$('add-close').onclick = () => { $('add').hidden = true; };
$('add-reveal').onclick = (e) => { e.preventDefault(); reveal(); };
$('add-tabs').onclick = (e) => {
  if (!e.target.dataset.way) return;
  addWay = e.target.dataset.way;
  document.querySelectorAll('#add-tabs button').forEach((b) => b.classList.toggle('on', b === e.target));
  ['link', 'file', 'folder'].forEach((w) => ($('way-' + w).hidden = w !== addWay));
  $('add-meta').hidden = addWay === 'folder';
};
$('add-urls').oninput = () => {
  if ($('add-name').value) return;
  const f = ($('add-urls').value.split(/\s+/)[0] || '').split('/').pop() || '';
  $('add-name').value = f.replace(/\.(gguf|bin|safetensors)$/i, '').replace(/[-_.](Q\d.*|q\d.*|f16|F16|fp16)$/,'').replace(/[-_]/g, ' ');
  if (/whisper|ggml-.*\.bin/i.test(f)) $('add-kind').value = 'voice';
  else if (/diffusion|sd[_-]|flux|sdxl/i.test(f)) $('add-kind').value = 'image';
};
$('add-pick').onclick = async () => {
  if (!tauri) return toast('Choosing files needs the app', true);
  picked = await tauri.invoke('pick_files');
  $('add-picked').textContent = picked.length ? picked.map((p) => p.split('/').pop()).join(', ') : 'No files chosen';
  if (picked.length && !$('add-name').value) $('add-name').value = picked[0].split('/').pop().replace(/\.(gguf|bin|safetensors)$/i, '').replace(/[-_]/g, ' ');
};
$('add-go').onclick = async () => {
  const name = $('add-name').value.trim(), kind = $('add-kind').value;
  if (!name) return toast('Give the model a name', true);
  try {
    if (addWay === 'link') {
      const urls = $('add-urls').value.split(/\s+/).filter(Boolean);
      await api('/api/import/url', { body: { name, kind, urls } });
      toast(`Downloading ${name}. It appears in Library when it's done.`);
    } else {
      if (!picked.length) return toast('Choose the model files first', true);
      await api('/api/import/file', { body: { name, kind, paths: picked } });
      toast(`Added ${name}`);
    }
    $('add').hidden = true; $('add-urls').value = ''; $('add-name').value = ''; picked = []; $('add-picked').textContent = 'No files chosen';
    ensureEngine(kind === 'voice' ? 'transcribe' : kind).catch(() => {});
    refresh();
  } catch (e) { toast(e.message, true); }
};

// ---------- studio ----------
function fill(sel, kind) {
  const ms = state.library.filter((m) => m.kind === kind);
  const keep = sel.value;
  const html = ms.length ? ms.map((m) => `<option value="${esc(m.id)}">${esc(m.name)}</option>`).join('') : `<option value="">No model yet: get one in Discover</option>`;
  if (sel.dataset.html !== html) { sel.innerHTML = html; sel.dataset.html = html; if (keep) sel.value = keep; }
}
function renderStudioSelects() { fill($('speak-model'), 'speak'); fill($('tr-model'), 'transcribe'); fill($('img-model'), 'image'); fill($('vid-model'), 'video'); }

async function busy(btn, out, fn) {
  const label = btn.textContent;
  btn.disabled = true; btn.textContent = 'Working…'; out.className = 'out'; out.textContent = '';
  try { await fn(); } catch (e) { out.className = 'out err'; out.textContent = e.message; }
  btn.disabled = false; btn.textContent = label;
}
$('speak-go').onclick = () => busy($('speak-go'), $('speak-out'), async () => {
  await ensureEngine('speak');
  const r = await api('/v1/audio/speech', { body: { input: $('speak-text').value, model: $('speak-model').value }, raw: true });
  const a = $('speak-audio');
  a.src = URL.createObjectURL(await r.blob()); a.hidden = false; a.play();
});
$('tr-file').onchange = () => { $('drop-text').textContent = $('tr-file').files[0]?.name || 'Choose an audio file'; };
$('tr-go').onclick = () => busy($('tr-go'), $('tr-out'), async () => {
  const f = $('tr-file').files[0];
  if (!f) throw new Error('Choose an audio file first');
  await ensureEngine('transcribe');
  const fd = new FormData(); fd.append('file', f); fd.append('model', $('tr-model').value);
  $('tr-out').textContent = (await api('/v1/audio/transcriptions', { body: fd })).text || '(no speech found)';
});
$('img-go').onclick = () => busy($('img-go'), $('img-out'), async () => {
  await ensureEngine('image');
  const t0 = performance.now();
  const r = await api('/v1/images/generations', { body: { prompt: $('img-prompt').value || $('img-prompt').placeholder, size: $('img-size').value, model: $('img-model').value } });
  const img = document.createElement('img');
  img.src = 'data:image/png;base64,' + r.data[0].b64_json; img.title = 'Show in Finder';
  img.onclick = () => api('/api/reveal', { body: { path: r.data[0].path } });
  $('gallery').prepend(img);
  $('img-out').textContent = `Made in ${((performance.now() - t0) / 1000).toFixed(1)} s. Saved in Model Launcher's outputs folder.`;
});

$('vid-go').onclick = () => busy($('vid-go'), $('vid-out'), async () => {
  await ensureEngine('video');
  const t0 = performance.now();
  $('vid-out').textContent = 'Making the clip. This takes a few minutes on a laptop…';
  const r = await api('/v1/videos/generations', { body: { prompt: $('vid-prompt').value || $('vid-prompt').placeholder, seconds: +$('vid-len').value, model: $('vid-model').value } });
  const v = document.createElement('video');
  v.src = 'data:video/webm;base64,' + r.data[0].b64_webm; v.controls = true; v.loop = true; v.autoplay = true; v.muted = true;
  $('vid-gallery').prepend(v);
  $('vid-out').textContent = `Made ${r.data[0].frames} frames at ${r.data[0].size} in ${((performance.now() - t0) / 1000).toFixed(0)} s.`;
});

// ---------- connect ----------
let codeLang = 'curl';
$('code-tabs').onclick = (e) => {
  if (!e.target.dataset.lang) return;
  codeLang = e.target.dataset.lang;
  document.querySelectorAll('#code-tabs button').forEach((b) => b.classList.toggle('on', b === e.target));
  renderConnect();
};
async function renderConnect() {
  const url = state.status?.api_url || API + '/v1';
  const model = state.library.find((m) => m.kind === 'chat')?.id || 'qwen3-vl-8b';
  $('api-url').textContent = url;
  $('api-example').textContent = {
    curl: `curl ${url}/chat/completions \\\n  -H "Content-Type: application/json" \\\n  -d '{"model": "${model}", "messages": [{"role": "user", "content": "Hello!"}]}'`,
    python: `from openai import OpenAI\n\nclient = OpenAI(base_url="${url}", api_key="local")\nr = client.chat.completions.create(\n    model="${model}",\n    messages=[{"role": "user", "content": "Hello!"}],\n)\nprint(r.choices[0].message.content)`,
    js: `import OpenAI from "openai";\n\nconst client = new OpenAI({ baseURL: "${url}", apiKey: "local" });\nconst r = await client.chat.completions.create({\n  model: "${model}",\n  messages: [{ role: "user", content: "Hello!" }],\n});\nconsole.log(r.choices[0].message.content);`,
  }[codeLang];
  if (!tauri) return;
  const info = await tauri.invoke('mcp_info');
  $('mcp-config').textContent = JSON.stringify(info.config, null, 2);
  $('clients').innerHTML = info.clients.map((c) => `<div class="client">
    <div class="icon t-gray">${esc(initials(c.name))}</div>
    <div class="client-name"><b>${esc(c.name)}</b><span class="muted">${c.connected ? 'Connected' : c.installed ? 'Not connected' : 'Not installed'}</span></div>
    ${c.connected ? '<span class="badge great">✓</span>' : `<button class="btn primary sm" onclick="connectClient('${c.id}')" ${c.installed ? '' : 'disabled'}>Connect</button>`}</div>`).join('');
}
async function connectClient(id) {
  try { toast(await tauri.invoke('connect_client', { id })); renderConnect(); } catch (e) { toast(String(e), true); }
}

// ---------- settings ----------
let settingsDrawn = false;
function renderSettings() {
  const s = state.status.settings, sys = state.status.system;
  $('s-budget').max = Math.max(2, Math.floor(sys.ram_total_gb - 2));
  $('s-threads').max = sys.cpus;
  if (!settingsDrawn || document.activeElement?.closest?.('#page-settings') == null) {
    $('s-budget').value = s.memory_budget_gb;
    $('s-ctx').value = String(s.context_size);
    $('s-idle').value = String(s.idle_unload_minutes);
    $('s-max').value = String(s.max_loaded);
    $('s-threads').value = s.threads;
    $('s-gpu').checked = s.use_gpu;
    $('s-low').checked = s.low_priority;
    $('s-web').checked = s.web_access;
    $('s-disk').checked = s.disk_memory;
    $('s-update').checked = s.auto_update;
    settingsDrawn = true;
  }
  $('v-budget').textContent = `${$('s-budget').value} GB`;
  $('v-threads').textContent = $('s-threads').value === '0' ? 'Auto' : $('s-threads').value;
  $('max-note').textContent = `Auto is ${state.status.max_loaded === 1 && !s.max_loaded ? '1 on this Mac' : state.status.max_loaded} (1 under 24 GB of RAM, 2 at 24 GB, 3 at 32 GB, 4 at 48 GB or more).`;
  $('ram-note').textContent = `This Mac has ${gb(sys.ram_total_gb)}; ${gb(sys.ram_available_gb)} is free right now. Disk: ${gb(sys.disk_free_gb)} free.`;
  const names = { llama_server: ['llama.cpp', 'Chat'], llama_tts: ['llama.cpp TTS', 'Text to speech'], whisper_cli: ['whisper.cpp', 'Speech to text'], sd_cli: ['stable-diffusion.cpp', 'Images'] };
  $('engines').innerHTML = state.status.engines.map((e) => `<div class="engine"><div><b>${names[e.engine][0]}</b><div class="muted">${names[e.engine][1]}</div></div>${e.path ? '<span class="badge great">Installed</span>' : `<button class="btn primary" onclick="installEngine('${e.engine}')">Install</button>`}</div>`).join('');
  renderDrives();
  renderUpdate();
  $('events').innerHTML = state.status.events.map((e) => `<li>${esc(e)}</li>`).join('') || '<li>Nothing yet.</li>';
}
function renderUpdate() {
  const u = state.status.update || {};
  const mac = navigator.userAgent.includes('Mac');
  $('update-info').innerHTML = u.latest
    ? `Version ${esc(u.current)}. <b>${esc(u.latest)} is available</b>${u.ready ? ' and downloaded.' : u.downloading ? ', downloading…' : '.'} <a href="#" data-link="${esc(u.page)}">What's new</a>`
    : `Version ${esc(u.current || state.status.version)}. ${u.checked_at ? `Up to date (checked ${esc(u.checked_at.slice(11, 16))}).` : 'Not checked yet.'}${u.error ? ` <span style="color:var(--red)">${esc(u.error)}</span>` : ''}`;
  $('update-action').innerHTML = u.ready ? '<button class="btn primary" onclick="installUpdate()">Install and reopen</button>'
    : u.latest && !mac ? `<button class="btn primary" data-link="${esc(u.page)}">Download ${esc(u.latest)}</button>` : '';
}
async function installUpdate() {
  if (!(await confirmDelete('Install the update now?', 'Loaded models are unloaded, Model Launcher quits, updates itself and opens again in a few seconds.', 'Install', false))) return;
  try { await api('/api/update/install', { body: {} }); toast('Installing…'); } catch (e) { toast(e.message, true); }
}
$('update-check').onclick = async () => {
  try { const u = await api('/api/update/check', { body: {} }); toast(u.latest ? `${u.latest} is available` : "You're up to date"); refresh(); } catch (e) { toast(e.message, true); }
};
let drivesAt = 0;
async function renderDrives() {
  const mv = state.status.models_move;
  if (mv && !mv.finished && !mv.error) {
    $('drives').innerHTML = `<div class="drive"><div style="flex:1"><b>Moving models to ${esc(mv.to)}</b><div class="progress"><i style="width:${mv.total_gb ? (100 * mv.done_gb / mv.total_gb).toFixed(1) : 2}%"></i></div><div class="muted">${gb(mv.done_gb)} of ${gb(mv.total_gb)}. Models can't be used until it's done.</div></div></div>`;
    return;
  }
  if (Date.now() - drivesAt < 5000 && state.drives) return drawDrives(mv);
  drivesAt = Date.now();
  try { state.drives = await api('/api/drives'); } catch { return; }
  drawDrives(mv);
}
function drawDrives(mv) {
  const err = state.status.models_drive_error;
  $('drives').innerHTML = (err ? `<div class="muted" style="color:var(--red)">${esc(err)}</div>` : '') + (mv?.error ? `<div class="muted" style="color:var(--red)">The move stopped: ${esc(mv.error)}. Your models are still in the old place.</div>` : '') +
    state.drives.drives.map((d) => `<div class="drive"><div style="flex:1"><b>${esc(d.name)}</b>${d.external ? '' : ' <span class="muted">(internal)</span>'}<div class="muted">${gb(d.free_gb)} free · ${esc(d.path)}</div></div>
      ${d.current ? '<span class="badge great">Models are here</span>' : `<button class="btn" onclick="moveModels('${esc(d.path)}', '${esc(d.name)}')">Move models here</button>`}</div>`).join('') +
    (state.drives.drives.length < 2 ? '<div class="muted">Plug in an external drive and it shows up here.</div>' : '');
}
async function moveModels(path, name) {
  const used = state.library.reduce((t, m) => t + m.size_gb, 0);
  if (!(await confirmDelete(`Move models to ${name}?`, `About ${gb(used)} is copied to ${path}, checked, and then removed from where it is now. Loaded chat models are unloaded first, and models can't be used until it's done.${name === 'This Mac' ? '' : '\n\nKeep the drive plugged in to use your models.'}`, 'Move models', false))) return;
  try { await api('/api/models/move', { body: { path } }); drivesAt = 0; toast('Moving models…'); refresh(); } catch (e) { toast(e.message, true); }
}
async function installEngine(engine) {
  toast('Installing… this can take a minute');
  try { toast((await api('/api/engines/install', { body: { engine } })).message); } catch (e) { toast(e.message, true); }
  refresh();
}
let saveTimer;
function saveSettings() {
  clearTimeout(saveTimer);
  $('v-budget').textContent = `${$('s-budget').value} GB`;
  $('v-threads').textContent = $('s-threads').value === '0' ? 'Auto' : $('s-threads').value;
  saveTimer = setTimeout(async () => {
    try {
      await api('/api/settings', { body: {
        memory_budget_gb: +$('s-budget').value, context_size: +$('s-ctx').value, idle_unload_minutes: +$('s-idle').value, max_loaded: +$('s-max').value,
        threads: +$('s-threads').value, use_gpu: $('s-gpu').checked, low_priority: $('s-low').checked, web_access: $('s-web').checked, disk_memory: $('s-disk').checked, auto_update: $('s-update').checked } });
      state.catalog = await api('/api/catalog');
      toast('Saved');
      refresh();
    } catch (e) { toast(e.message, true); }
  }, 350);
}
['s-budget', 's-ctx', 's-idle', 's-max', 's-threads', 's-gpu', 's-low', 's-web', 's-disk', 's-update'].forEach((id) => $(id).addEventListener('input', saveSettings));

refresh();
setInterval(refresh, 2000);
