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
    throw new Error(msg);
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

function confirmDelete(title, text) {
  return new Promise((resolve) => {
    $('modal-title').textContent = title;
    $('modal-text').textContent = text;
    $('modal').hidden = false;
    const done = (v) => { $('modal').hidden = true; resolve(v); };
    $('modal-yes').onclick = () => done(true);
    $('modal-no').onclick = () => done(false);
  });
}

const FIT = { great: ['great', 'Runs great'], ok: ['ok', 'Fits'], over_budget: ['over', 'Over budget'], too_big: ['too', 'Too big'] };
const fitBadge = (f) => { const [c, t] = FIT[f] || ['', f]; return `<span class="badge ${c}">${t}</span>`; };
const KIND = { chat: ['chat', 'Chat'], transcribe: ['voice', 'Speech to text'], speak: ['voice', 'Text to speech'], image: ['image', 'Image'] };
const ENGINE_FOR = { chat: 'llama_server', transcribe: 'whisper_cli', speak: 'llama_tts', image: 'sd_cli' };
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
  $('meter-fill').style.width = Math.min(100, (s.used_gb / budget) * 100) + '%';
  $('meter-text').textContent = `${gb(s.used_gb)} of ${gb(budget)} budget`;
}

function renderBanner() {
  const r = state.status.running, j = state.status.job;
  let html = '';
  if (r) {
    html += `<div class="card dark">
      <span class="live ${r.ready ? '' : 'loading'}"></span>
      <div style="flex:1"><div class="card-title">${esc(r.name)} ${r.ready ? 'is loaded' : 'is loading…'}</div>
      <div class="muted">Using ${gb(r.memory_gb)} · ${r.vision ? 'reads images · ' : ''}idle ${Math.floor(r.idle_secs / 60)} min</div></div>
      <button class="btn primary" onclick="openChat('${esc(r.id)}')" ${r.ready ? '' : 'disabled'}>Open chat</button>
      <button class="btn icon-only" onclick="unload()">Unload</button></div>`;
  }
  if (j) html += `<div class="card"><span class="live loading"></span><div>${esc(j.label)}… using ${gb(j.memory_gb)}</div></div>`;
  $('banner').innerHTML = html;
}

function modelCard(m) {
  const running = state.status.running?.id === m.id;
  const [cls, label] = KIND[m.kind];
  const main = m.kind === 'chat'
    ? `<button class="btn ${running ? 'dark' : 'primary'}" onclick="openChat('${esc(m.id)}')" ${state.loading === m.id ? 'disabled' : ''}>${state.loading === m.id ? 'Loading…' : running ? 'Open chat' : 'Chat'}</button>`
    : `<button class="btn primary" onclick="tryStudio()">Try in Studio</button>`;
  return `<div class="card model ${running ? 'dark' : ''}">
    <div class="model-head"><div class="icon ${cls}">${esc(initials(m.name))}</div>
      <div><h3>${esc(m.name)}</h3><div class="muted">${esc(m.maker || 'Added by you')} · ${label}</div></div></div>
    <div class="pills"><span class="pill">${gb(m.size_gb)} on disk</span><span class="pill">needs ~${gb(m.need_gb)}</span>${m.vision ? '<span class="pill">Reads images</span>' : ''}${fitBadge(m.fit)}</div>
    <div class="spacer"></div>
    <div class="actions">${main}
      ${running ? `<button class="btn icon-only" onclick="unload()">Unload</button>` : ''}
      <span class="spacer"></span>
      <button class="btn icon-only" title="Show in Finder" onclick="reveal('${esc(m.id)}')">Folder</button>
      <button class="btn icon-only" title="Delete" onclick="removeModel('${esc(m.id)}')">Delete</button></div>
  </div>`;
}

function renderLibrary() {
  const lib = state.library;
  if (!lib.length) {
    $('library').innerHTML = `<div class="empty"><h3>No models yet</h3><p>Get one from Discover. Look for <span class="badge great">Runs great</span>.</p><br><button class="btn primary" onclick="document.querySelector('[data-tab=discover]').click()">Discover models</button></div>`;
    return;
  }
  const groups = [['Chat', (m) => m.kind === 'chat'], ['Voice', (m) => m.kind === 'transcribe' || m.kind === 'speak'], ['Image', (m) => m.kind === 'image']];
  $('library').innerHTML = groups
    .map(([name, f]) => { const ms = lib.filter(f); return ms.length ? `<h2 class="section">${name}</h2><div class="grid">${ms.map(modelCard).join('')}</div>` : ''; })
    .join('');
}

function renderCatalog() {
  const f = state.filter;
  const dl = state.status?.downloads || {};
  const list = state.catalog.filter((m) => f === 'all' || (f === 'chat' && m.kind === 'chat') || (f === 'voice' && (m.kind === 'transcribe' || m.kind === 'speak')) || (f === 'image' && m.kind === 'image'));
  $('catalog').innerHTML = list.map((m) => {
    const [cls, label] = KIND[m.kind];
    const d = dl[m.id];
    let action;
    if (m.installed) action = `<span class="pill">Installed</span>`;
    else if (d && !d.finished && !d.error) action = `<div style="flex:1"><div class="progress"><i style="width:${(100 * d.done_gb / d.total_gb).toFixed(1)}%"></i></div><div class="muted">${gb(d.done_gb)} of ${gb(d.total_gb)}</div></div><button class="btn icon-only" onclick="cancelDl('${m.id}')">Cancel</button>`;
    else action = `<button class="btn primary" onclick="download('${m.id}')" ${m.fit === 'too_big' ? 'disabled' : ''}>Download · ${gb(m.size_gb)}</button>${d?.error ? `<span class="muted">${esc(d.error)}</span>` : ''}`;
    return `<div class="card model">
      <div class="model-head"><div class="icon ${cls}">${esc(initials(m.name))}</div><div><h3>${esc(m.name)}</h3><div class="muted">${esc(m.maker)} · ${label}</div></div></div>
      <p class="blurb">${esc(m.blurb)}</p>
      <div class="pills">${m.vision ? '<span class="pill">Reads images</span>' : ''}<span class="pill">needs ~${gb(m.need_gb)}</span>${fitBadge(m.fit)}</div>
      <div class="actions">${action}</div></div>`;
  }).join('');
}

// ---------- actions ----------
async function openChat(id) {
  try {
    const m = state.library.find((x) => x.id === id);
    await ensureEngine('chat');
    state.loading = id;
    render();
    const r = await api('/api/launch', { body: { id } });
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
async function unload() { await api('/api/stop', { body: {} }); refresh(); }
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
function tryStudio() { document.querySelector('[data-tab=studio]').click(); }
$('open-models').onclick = (e) => { e.preventDefault(); reveal(); };
document.addEventListener('click', (e) => {
  const link = e.target.closest('[data-link]');
  if (link) { e.preventDefault(); tauri ? tauri.invoke('open_url', { url: link.dataset.link }) : window.open(link.dataset.link); }
  const copy = e.target.closest('[data-copy]');
  if (copy) { navigator.clipboard.writeText($(copy.dataset.copy).textContent); toast('Copied'); }
});

// ---------- studio ----------
function fill(sel, kind) {
  const ms = state.library.filter((m) => m.kind === kind);
  const keep = sel.value;
  const html = ms.length ? ms.map((m) => `<option value="${esc(m.id)}">${esc(m.name)}</option>`).join('') : `<option value="">No model yet: get one in Discover</option>`;
  if (sel.dataset.html !== html) { sel.innerHTML = html; sel.dataset.html = html; if (keep) sel.value = keep; }
}
function renderStudioSelects() { fill($('speak-model'), 'speak'); fill($('tr-model'), 'transcribe'); fill($('img-model'), 'image'); }

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

// ---------- connect ----------
async function renderConnect() {
  const url = state.status?.api_url || API + '/v1';
  $('api-url').textContent = url;
  $('api-example').textContent = `curl ${url}/chat/completions \\\n  -H "Content-Type: application/json" \\\n  -d '{"model": "${state.library.find((m) => m.kind === 'chat')?.id || 'qwen3-vl-8b'}", "messages": [{"role": "user", "content": "Hello!"}]}'\n\n# Python\nfrom openai import OpenAI\nclient = OpenAI(base_url="${url}", api_key="local")`;
  if (!tauri) return;
  const info = await tauri.invoke('mcp_info');
  $('mcp-config').textContent = JSON.stringify(info.config, null, 2);
  $('clients').innerHTML = info.clients.map((c) => `<div class="client"><span>${esc(c.name)}</span>${c.connected ? '<span class="badge great">Connected</span>' : `<button class="btn primary" onclick="connectClient('${c.id}')">Connect</button>`}</div>`).join('');
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
    $('s-threads').value = s.threads;
    $('s-gpu').checked = s.use_gpu;
    $('s-low').checked = s.low_priority;
    settingsDrawn = true;
  }
  $('v-budget').textContent = `${$('s-budget').value} GB`;
  $('v-threads').textContent = $('s-threads').value === '0' ? 'Auto' : $('s-threads').value;
  $('ram-note').textContent = `This Mac has ${gb(sys.ram_total_gb)}; ${gb(sys.ram_available_gb)} is free right now. Disk: ${gb(sys.disk_free_gb)} free.`;
  const names = { llama_server: ['llama.cpp', 'Chat'], llama_tts: ['llama.cpp TTS', 'Text to speech'], whisper_cli: ['whisper.cpp', 'Speech to text'], sd_cli: ['stable-diffusion.cpp', 'Images'] };
  $('engines').innerHTML = state.status.engines.map((e) => `<div class="engine"><div><b>${names[e.engine][0]}</b><div class="muted">${names[e.engine][1]}</div></div>${e.path ? '<span class="badge great">Installed</span>' : `<button class="btn primary" onclick="installEngine('${e.engine}')">Install</button>`}</div>`).join('');
  $('events').innerHTML = state.status.events.map((e) => `<li>${esc(e)}</li>`).join('') || '<li>Nothing yet.</li>';
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
        memory_budget_gb: +$('s-budget').value, context_size: +$('s-ctx').value, idle_unload_minutes: +$('s-idle').value,
        threads: +$('s-threads').value, use_gpu: $('s-gpu').checked, low_priority: $('s-low').checked } });
      state.catalog = await api('/api/catalog');
      toast('Saved');
      refresh();
    } catch (e) { toast(e.message, true); }
  }, 350);
}
['s-budget', 's-ctx', 's-idle', 's-threads', 's-gpu', 's-low'].forEach((id) => $(id).addEventListener('input', saveSettings));

refresh();
setInterval(refresh, 2000);
