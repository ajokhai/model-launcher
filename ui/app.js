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
const LOGOS = new Set(['Qwen', 'google', 'meta-llama', 'openai', 'stabilityai', 'OuteAI', 'Wan-AI']);
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
  const frac = s.used_gb / budget;
  $('meter-fill').style.width = Math.min(100, frac * 100) + '%';
  $('meter-fill').className = frac > 1 ? 'over' : frac > 0.85 ? 'warn' : '';
  $('meter-text').textContent = `RAM ${gb(s.used_gb)} / ${gb(budget)} · Disk ${gb(s.system.disk_free_gb)} free`;
}

function renderBanner() {
  const rs = state.status.running || [], j = state.status.job, max = state.status.max_loaded;
  let html = '';
  if (rs.length) {
    html += `<div class="card dark loaded"><div class="loaded-head"><span class="live"></span><b>${rs.length} of ${max} loaded</b><span class="muted">Models stay loaded while they fit your RAM budget; the least recently used makes room.</span></div>
      ${rs.map((r) => `<div class="loaded-row"><span class="live ${r.ready ? '' : 'loading'}"></span>
        <div style="flex:1"><b>${esc(r.name)}</b> <span class="muted">${r.ready ? '' : 'loading… · '}${gb(r.memory_gb)} · ${r.vision ? 'reads images · ' : ''}idle ${Math.floor(r.idle_secs / 60)} min</span></div>
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
  const catalogIds = new Set(state.catalog.map((m) => m.id));
  const rows = Object.entries(dl).filter(([id, d]) => !catalogIds.has(id) && (!d.finished || d.error));
  $('pending').innerHTML = rows.map(([id, d]) => `<div class="card pending-row"><div style="flex:1"><b>${esc(id)}</b>
    ${d.error ? `<div class="muted" style="color:var(--red)">${esc(d.error)}</div>` : `<div class="progress"><i style="width:${d.total_gb ? (100 * d.done_gb / d.total_gb).toFixed(1) : 2}%"></i></div><div class="muted">${gb(d.done_gb)} of ${d.total_gb > .01 ? gb(d.total_gb) : '…'}</div>`}</div>
    ${d.error ? '' : `<button class="btn icon-only" onclick="cancelDl('${esc(id)}')">Cancel</button>`}</div>`).join('');
}

function renderLibrary() {
  renderPending();
  const lib = state.library;
  if (!lib.length) {
    $('library').innerHTML = `<div class="empty"><h3>No models yet</h3><p>Get one from Discover. Look for <span class="badge great">Runs great</span>.</p><br><button class="btn primary" onclick="document.querySelector('[data-tab=discover]').click()">Discover models</button></div>`;
    return;
  }
  const groups = [['Chat', (m) => m.kind === 'chat'], ['Voice', (m) => m.kind === 'transcribe' || m.kind === 'speak'], ['Image', (m) => m.kind === 'image'], ['Video', (m) => m.kind === 'video']];
  $('library').innerHTML = groups
    .map(([name, f]) => { const ms = lib.filter(f); return ms.length ? `<h2 class="section">${name}</h2><div class="grid">${ms.map(modelCard).join('')}</div>` : ''; })
    .join('');
}

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
$('add-open').onclick = () => { $('add').hidden = false; };
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
    settingsDrawn = true;
  }
  $('v-budget').textContent = `${$('s-budget').value} GB`;
  $('v-threads').textContent = $('s-threads').value === '0' ? 'Auto' : $('s-threads').value;
  $('max-note').textContent = `Auto is ${state.status.max_loaded === 1 && !s.max_loaded ? '1 on this Mac' : state.status.max_loaded} (1 under 24 GB of RAM, 2 at 24 GB, 3 at 32 GB, 4 at 48 GB or more).`;
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
        memory_budget_gb: +$('s-budget').value, context_size: +$('s-ctx').value, idle_unload_minutes: +$('s-idle').value, max_loaded: +$('s-max').value,
        threads: +$('s-threads').value, use_gpu: $('s-gpu').checked, low_priority: $('s-low').checked } });
      state.catalog = await api('/api/catalog');
      toast('Saved');
      refresh();
    } catch (e) { toast(e.message, true); }
  }, 350);
}
['s-budget', 's-ctx', 's-idle', 's-max', 's-threads', 's-gpu', 's-low'].forEach((id) => $(id).addEventListener('input', saveSettings));

refresh();
setInterval(refresh, 2000);
