// SPDX-License-Identifier: MPL-2.0
// Pages of the admin UI. Each page: {title, icon, feature?, render(main, ctx)}.
// Data only from the documented API (internal/server/openapi.yaml; users, sharing and SSH
// from DIST-018 when /api/v1/openapi.yaml lists them).

import { ApiError, del, get, post, put, waitForService } from './api.js';
import { bytes, dateTime, duration, num, t } from './i18n.js';
import { askAdminPassword, chip, confirmDialog, dialog, errText, every, fail, h, kv, meter, stopTimer, switchEl, toast } from './ui.js';

const RELEASES_URL = 'https://github.com/DenisHumen/kinolune/releases';

const head = (title, sub, ...actions) => h('div.page-head', h('div', h('h1', title), sub ? h('p.muted', sub) : null), actions.length ? h('div.row', actions) : null);
const panel = (title, ...kids) => h('section.panel', title ? h('h3', title) : null, ...kids);
const tryGet = (p) => get(p).catch((e) => (e instanceof ApiError && e.status !== 401 ? { __error: e } : Promise.reject(e)));
const failed = (x) => x && x.__error;
const errBox = (x) => h('div.alert.bad', errText(x.__error));

function activeChip(u) {
  if (u.active_state === 'active') return chip(t('svc.active'), 'ok');
  if (u.active_state === 'failed') return chip(t('svc.failed'), 'bad');
  if (u.active_state === 'activating' || u.active_state === 'reloading') return chip(t('svc.starting'), 'warn');
  if (u.load_state === 'not-found') return chip(t('svc.missing'));
  return chip(t('svc.inactive'));
}

function healthChip(hl) {
  const s = (hl && hl.status) || 'unknown';
  return chip(t(`disk.health.${s}`), s === 'ok' ? 'ok' : s === 'warning' ? 'warn' : s === 'failing' ? 'bad' : '');
}

const updateStateChip = (u) => {
  const s = u && u.state;
  const kind = { 'up-to-date': 'ok', available: 'warn', downloaded: 'warn', staged: 'warn', 'rollback-queued': 'warn' }[s] || '';
  return chip(t(`upd.state.${s || 'unknown'}`), kind);
};

/** Reboot-like action: waits for the machine to come back, then reloads to the login page. */
async function waitReboot(main, text) {
  stopTimer(); // the page's own refresh would only show errors while the server is down
  const status = h('p.muted.busy', text);
  main.replaceChildren(head(t('reboot.title')), panel(null, status, h('p.caption', t('reboot.hint'))));
  const back = await waitForService({
    onTick: ({ up, wentDown }) => { status.textContent = !wentDown ? text : up ? t('reboot.back') : t('reboot.down'); },
  });
  if (back) location.reload();
  else status.replaceWith(h('div.alert.warn', t('reboot.timeout')));
}

// --- dashboard ---------------------------------------------------------------------------

async function dashboard(main) {
  const body = h('div.grid');
  main.append(head(t('dash.title'), t('dash.subtitle')), body);
  const draw = async () => {
    const [sys, disks, svcs, upd, net] = await Promise.all([
      tryGet('/api/v1/system'), tryGet('/api/v1/disks'), tryGet('/api/v1/services'), tryGet('/api/v1/system/update'), tryGet('/api/v1/network'),
    ]);
    const cards = [];
    if (failed(sys)) cards.push(panel(t('dash.system'), errBox(sys)));
    else {
      const load = (sys.load || [0, 0, 0]);
      const cpuFrac = sys.cpus ? load[0] / sys.cpus : 0;
      const used = sys.memory_total ? (sys.memory_total - sys.memory_available) : 0;
      cards.push(panel(t('dash.system'), kv([
        [t('dash.host'), sys.hostname],
        [t('dash.os'), sys.os_version ? `Kinolune OS ${sys.os_version}` : '—'],
        [t('dash.kernel'), sys.kernel],
        [t('dash.uptime'), duration(sys.uptime_seconds)],
      ])));
      cards.push(panel(t('dash.cpu'),
        h('div.big.num', { 'data-testid': 'cpu' }, `${num(Math.min(cpuFrac, 9.99) * 100)} %`),
        meter(cpuFrac, 0.7, 0.95),
        h('p.caption', t('dash.load', { a: num(load[0], 2), b: num(load[1], 2), c: num(load[2], 2), n: sys.cpus || '?' }))));
      cards.push(panel(t('dash.ram'),
        h('div.big.num', { 'data-testid': 'ram' }, bytes(used)),
        meter(sys.memory_total ? used / sys.memory_total : 0),
        h('p.caption', t('dash.ram_of', { total: bytes(sys.memory_total), free: bytes(sys.memory_available) }))));
    }
    if (failed(disks)) cards.push(panel(t('dash.disks'), errBox(disks)));
    else {
      const drives = disks.drives || [];
      cards.push(panel(t('dash.disks'),
        drives.length ? h('div.stack', drives.map((d) => h('div.row.between',
          h('div', h('div', [d.vendor, d.model].filter(Boolean).join(' ') || d.id), h('div.caption', [bytes(d.size), d.system ? t('disk.system') : null].filter(Boolean).join(' · '))),
          h('div.row', d.health && Number.isFinite(d.health.temperature_c) && d.health.temperature_c > 0 ? h('span.num', { 'data-testid': 'disk-temp' }, `${num(d.health.temperature_c)} °C`) : null, healthChip(d.health)))))
          : h('p.muted', t('disk.none')),
        h('p.caption', t('dash.temps_note')),
        h('a', { href: '#/disks' }, t('dash.more'))));
    }
    if (failed(upd)) cards.push(panel(t('dash.updates'), errBox(upd)));
    else {
      cards.push(panel(t('dash.updates'),
        h('div.row', updateStateChip(upd), upd.operation ? chip(t(`upd.op.${upd.operation.kind}`), 'warn') : null),
        kv([
          [t('upd.current'), upd.current_version],
          upd.available ? [t('upd.available'), upd.available.version || upd.available.digest] : null,
          [t('upd.last_check'), dateTime(upd.last_check)],
        ]),
        h('a', { href: '#/updates', 'data-testid': 'dash-updates' }, t('dash.more'))));
    }
    if (!failed(svcs)) {
      const list = svcs.services || [];
      const bad = list.filter((u) => u.active_state === 'failed');
      cards.push(panel(t('dash.services'),
        h('div.big.num', `${list.filter((u) => u.active_state === 'active').length} / ${list.length}`),
        h('p.caption', t('dash.services_active')),
        bad.length ? h('div.stack', bad.map((u) => h('div.row.between', h('span', u.title || u.name), activeChip(u)))) : null,
        h('a', { href: '#/system' }, t('dash.more'))));
    }
    if (!failed(net)) {
      const devs = (net.devices || []).filter((d) => (d.ipv4 || []).length);
      cards.push(panel(t('dash.network'),
        h('div.row', chip(t(`net.conn.${net.connectivity || 'unknown'}`), net.connectivity === 'full' ? 'ok' : net.connectivity === 'none' ? 'bad' : 'warn')),
        kv(devs.map((d) => [d.interface, (d.ipv4 || []).join(', ')])),
        h('a', { href: '#/network' }, t('dash.more'))));
    }
    body.replaceChildren(...cards);
  };
  await draw();
  every(5000, () => draw().catch(fail));
}

// --- updates -----------------------------------------------------------------------------

async function updates(main) {
  const body = h('div.stack');
  main.append(head(t('upd.title'), t('upd.subtitle'), h('a.btn', { href: RELEASES_URL, target: '_blank', rel: 'noopener noreferrer' }, t('upd.notes'))), body);
  let busyUntil = 0;
  let last = '';

  const act = async (fn, okText) => {
    busyUntil = Date.now() + 2500;
    try {
      await fn();
      if (okText) toast(okText, 'ok');
    } catch (e) {
      if (e instanceof ApiError && (e.code === 'busy' || e.status === 409)) toast(errText(e) || t('upd.busy'), 'bad');
      else fail(e);
    }
    await draw(true);
  };

  const inhibitorsNote = async () => {
    try {
      const p = await get('/api/v1/power');
      const list = (p.inhibitors || []).filter((i) => i.mode === 'block' || /alpha|kinolune|media/i.test(`${i.who} ${i.why}`));
      if (!list.length) return null;
      return h('div.alert.warn', t('power.inhibitors'), h('ul', list.map((i) => h('li', `${i.who}: ${i.why}`))));
    } catch { return null; }
  };

  const applyNow = async () => {
    const ok = await confirmDialog({ title: t('upd.apply_now'), text: t('upd.apply_now_text'), ok: t('upd.apply_now_ok'), extra: await inhibitorsNote(), testid: 'confirm-apply' });
    if (!ok) return;
    try {
      await post('/api/v1/system/update/apply', { when: 'now' });
    } catch (e) { fail(e); return draw(true); }
    await waitReboot(main, t('upd.applying'));
  };

  const rollback = async (s) => {
    const to = s.rollback && (s.rollback.version || s.rollback.digest);
    const ok = await confirmDialog({ title: t('upd.rollback'), text: t('upd.rollback_text', { v: to || '—' }), ok: t('upd.rollback_ok'), danger: true, extra: await inhibitorsNote(), testid: 'confirm-rollback' });
    if (!ok) return;
    try {
      await post('/api/v1/system/update/rollback', { reboot: true });
    } catch (e) { fail(e); return draw(true); }
    await waitReboot(main, t('upd.rolling_back'));
  };

  const settingsForm = (st) => {
    const channel = h('select', { 'data-testid': 'upd-channel' }, ['stable', 'beta'].map((c) => h('option', { value: c, selected: st.channel === c }, t(`upd.channel.${c}`))));
    const auto = h('select', { 'data-testid': 'upd-auto' }, ['off', 'download', 'install'].map((c) => h('option', { value: c, selected: st.auto === c }, t(`upd.auto.${c}`))));
    const win = h('input', { value: st.window || '', placeholder: '03:00-05:00', pattern: '^\\d{2}:\\d{2}-\\d{2}:\\d{2}$', 'data-testid': 'upd-window' });
    const save = h('button.primary', {
      type: 'button',
      'data-testid': 'upd-save',
      onclick: async () => {
        const body = {};
        if (channel.value !== st.channel) body.channel = channel.value;
        if (auto.value !== st.auto) body.auto = auto.value;
        if (win.value !== (st.window || '')) body.window = win.value;
        if (!Object.keys(body).length) return;
        if (body.channel && !(await confirmDialog({ title: t('upd.channel'), text: t('upd.channel_text'), ok: t('common.save') }))) return;
        await act(() => put('/api/v1/system/update/settings', body), t('common.saved'));
      },
    }, t('common.save'));
    return panel(t('upd.settings'),
      h('div.grid', h('label.field', t('upd.channel'), channel), h('label.field', t('upd.auto'), auto), h('label.field', t('upd.window'), win)),
      h('p.caption', t('upd.settings_hint')),
      h('div.row', save));
  };

  async function draw(force) {
    let s;
    try {
      s = await get('/api/v1/system/update/status');
    } catch (e) {
      if (e instanceof ApiError && e.status === 401) throw e;
      body.replaceChildren(h('div.alert.bad', errText(e)));
      return;
    }
    const key = JSON.stringify(s);
    if (!force && key === last) return;
    last = key;
    const u = s.update || {};
    const op = u.operation;
    const busy = Boolean(op) || Date.now() < busyUntil;
    const canInstall = ['available', 'downloaded', 'staged'].includes(u.state);
    const btn = (label, testid, onclick, kind = '') => h(`button${kind}`, { type: 'button', disabled: busy, 'data-testid': testid, onclick }, label);

    const status = panel(t('upd.status'),
      h('div.row', { 'data-testid': 'upd-state', 'data-state': u.state || 'unknown' }, updateStateChip(u),
        op ? h('span.muted.busy', { 'data-testid': 'upd-op' }, t(`upd.op.${op.kind}`)) : null,
        u.pending_apply ? chip(t('upd.pending'), 'warn') : null),
      kv([
        [t('upd.current'), h('span.version', { 'data-testid': 'upd-current' }, s.booted?.version || u.current_version || s.os_version || '—')],
        u.available ? [t('upd.available'), h('span.version', { 'data-testid': 'upd-available' }, u.available.version || (u.available.digest || '').slice(0, 19))] : null,
        s.staged ? [t('upd.staged'), s.staged.version || (s.staged.digest || '').slice(0, 19)] : null,
        s.rollback ? [t('upd.previous'), h('span', { 'data-testid': 'upd-previous' }, s.rollback.version || (s.rollback.digest || '').slice(0, 19))] : null,
        [t('upd.channel'), t(`upd.channel.${s.settings?.channel || 'stable'}`)],
        [t('upd.last_check'), dateTime(u.last_check)],
        s.next_window ? [t('upd.next_window'), dateTime(s.next_window)] : null,
        s.last_apply ? [t('upd.last_apply'), `${s.last_apply.version || s.last_apply.digest || ''} · ${dateTime(s.last_apply.at)}${s.last_apply.result ? ` · ${t(`upd.result.${s.last_apply.result}`)}` : ''}`] : null,
      ]),
      u.last_error ? h('div.alert.bad', { 'data-testid': 'upd-error' }, `${t(`upd.op.${u.last_error.op}`)}: ${u.last_error.message}`) : null,
      s.bootc_error ? h('div.alert.warn', s.bootc_error) : null,
      u.state === 'rollback-queued' ? h('div.alert.warn', t('upd.rollback_queued')) : null,
      h('div.steps',
        btn(t('upd.check'), 'upd-check', () => act(() => post('/api/v1/system/update/check'), t('upd.checking'))),
        u.state === 'available' ? btn(t('upd.download'), 'upd-download', () => act(() => post('/api/v1/system/update/download'), t('upd.downloading'))) : null,
        canInstall ? btn(t('upd.apply_now'), 'upd-apply', applyNow, '.primary') : null,
        canInstall && !u.pending_apply ? btn(t('upd.apply_window'), 'upd-apply-window', () => act(() => post('/api/v1/system/update/apply', { when: 'maintenance' }), t('upd.scheduled'))) : null,
        u.pending_apply ? btn(t('upd.cancel_window'), 'upd-cancel-window', () => act(() => del('/api/v1/system/update/apply'), t('common.done'))) : null,
        s.rollback ? btn(t('upd.rollback'), 'upd-rollback', () => rollback(s), '.danger') : null));
    body.replaceChildren(status, settingsForm(s.settings || {}), panel(t('upd.about'), h('p.muted', t('upd.about_text'))));
  }

  await draw(true);
  every(3000, () => draw().catch(fail));
}

// --- network -----------------------------------------------------------------------------

async function network(main) {
  const body = h('div.stack');
  main.append(head(t('net.title'), t('net.subtitle')), body);
  const draw = async () => {
    const [st, conns, fw] = await Promise.all([tryGet('/api/v1/network'), tryGet('/api/v1/network/connections'), tryGet('/api/v1/firewall')]);
    const parts = [];
    if (failed(st)) parts.push(errBox(st));
    else {
      parts.push(panel(t('net.devices'),
        h('div.row', chip(t(`net.conn.${st.connectivity || 'unknown'}`), st.connectivity === 'full' ? 'ok' : 'warn'),
          st.wireless_hardware_enabled ? switchEl({ label: t('net.wifi'), checked: st.wireless_enabled, testid: 'wifi', onChange: (on) => put('/api/v1/network/wireless', { enabled: on }).then(() => toast(t('common.saved'), 'ok')).catch((e) => { fail(e); throw e; }) }) : null),
        h('div.scroll', h('table.table',
          h('thead', h('tr', [t('net.iface'), t('net.type'), t('net.state'), 'IPv4', t('net.gateway'), 'DNS'].map((x) => h('th', x)))),
          h('tbody', (st.devices || []).filter((d) => d.type !== 'loopback').map((d) => h('tr',
            h('td', h('div', d.interface), h('div.caption.mono', d.hw_address || '')),
            h('td', d.type), h('td', d.state === 'activated' ? chip(t('net.up'), 'ok') : chip(d.state || '—')),
            h('td.mono', (d.ipv4 || []).join(', ') || '—'), h('td.mono', d.gateway4 || '—'), h('td.mono', (d.dns4 || []).join(', ') || '—'))))))));
    }
    if (!failed(conns)) {
      parts.push(panel(t('net.connections'), h('div.scroll', h('table.table',
        h('thead', h('tr', [t('net.name'), t('net.type'), 'IPv4', t('net.state'), ''].map((x) => h('th', x)))),
        h('tbody', (conns.connections || []).filter((c) => c.type !== 'loopback').map((c) => h('tr',
          h('td', c.id), h('td', c.type), h('td', c.ipv4_method === 'auto' ? 'DHCP' : c.ipv4_method || '—'),
          h('td', c.active ? chip(t('net.active'), 'ok') : chip(t('net.inactive'))),
          h('td.actions', c.active
            ? h('button.small', { type: 'button', onclick: async () => {
              if (!(await confirmDialog({ title: t('net.down'), text: t('net.down_text', { name: c.id }), ok: t('net.down'), danger: true }))) return;
              try { await post(`/api/v1/network/connections/${encodeURIComponent(c.uuid)}/down`); toast(t('common.done'), 'ok'); } catch (e) { fail(e); }
              draw().catch(fail);
            } }, t('net.down'))
            : h('button.small', { type: 'button', onclick: async () => {
              try { await post(`/api/v1/network/connections/${encodeURIComponent(c.uuid)}/up`); toast(t('common.done'), 'ok'); } catch (e) { fail(e); }
              draw().catch(fail);
            } }, t('net.up_btn'))))))))));
    }
    if (!failed(fw)) {
      const openings = (fw.openings || []).filter((o) => o.name !== 'cockpit' && o.name !== 'ssh');
      parts.push(panel(t('fw.title'),
        h('p.muted', t('fw.subtitle', { zone: fw.default_zone || '—' })),
        !fw.running ? h('div.alert.warn', t('fw.not_running')) : null,
        h('div.stack', openings.map((o) => h('div.row.between',
          h('div', h('div', t(`fw.${o.name}`)), h('div.caption.mono', [...(o.services || []), ...(o.ports || [])].join(', '))),
          switchEl({ checked: o.open, testid: `fw-${o.name}`, label: '', onChange: (on) => put(`/api/v1/firewall/openings/${o.name}`, { open: on }).then(() => toast(t('common.saved'), 'ok')).catch((e) => { fail(e); throw e; }) }))))));
    }
    body.replaceChildren(...parts);
  };
  await draw();
}

// --- users (DIST-018) --------------------------------------------------------------------

async function users(main) {
  const body = h('div.stack');
  const add = h('button.primary', { type: 'button', 'data-testid': 'user-add', onclick: () => addUser() }, t('users.add'));
  main.append(head(t('users.title'), t('users.subtitle'), add), body);

  const withAdmin = async (intro, fn, okText) => {
    const pw = await askAdminPassword(intro);
    if (!pw) return;
    try {
      await fn(pw);
      toast(okText || t('common.done'), 'ok');
    } catch (e) { fail(e); }
    await draw();
  };

  const addUser = async () => {
    const name = h('input', { autocomplete: 'off', autocapitalize: 'none', spellcheck: 'false', pattern: '^[a-z_][a-z0-9_-]{0,31}$', 'data-testid': 'new-user' });
    const full = h('input', { autocomplete: 'off', maxlength: 64 });
    const pass = h('input', { type: 'password', autocomplete: 'new-password', minlength: 8, 'data-testid': 'new-pass' });
    const admin = h('input', { type: 'checkbox' });
    const err = h('div.alert.bad', { hidden: true });
    const r = await dialog({
      title: t('users.add'),
      body: h('div.stack', h('label.field', t('users.login'), name), h('p.caption', t('users.login_hint')), h('label.field', t('users.full_name'), full),
        h('label.field', t('users.password'), pass), h('p.caption', t('users.password_hint')), h('label.row', admin, t('users.make_admin')), err),
      buttons: [{ label: t('common.cancel'), kind: 'ghost', value: null }, {
        label: t('common.continue'), kind: 'primary', testid: 'confirm', onClick: () => {
          const bad = !/^[a-z_][a-z0-9_-]{0,31}$/.test(name.value) ? t('users.bad_login') : pass.value.length < 8 ? t('users.short_password') : '';
          err.textContent = bad; err.hidden = !bad;
          return !bad;
        },
      }],
    });
    if (!r) return;
    await withAdmin(null, (pw) => post('/api/v1/users', { username: name.value, full_name: full.value || undefined, password: pass.value, admin: admin.checked, admin_password: pw }), t('users.created', { name: name.value }));
  };

  const setPassword = async (u) => {
    const pass = h('input', { type: 'password', autocomplete: 'new-password', minlength: 8 });
    const err = h('div.alert.bad', { hidden: true });
    const r = await dialog({
      title: t('users.set_password', { name: u.name }),
      body: h('div.stack', h('label.field', t('users.new_password'), pass), h('p.caption', t('users.password_hint')), err),
      buttons: [{ label: t('common.cancel'), kind: 'ghost', value: null }, {
        label: t('common.continue'), kind: 'primary', onClick: () => { const bad = pass.value.length < 8; err.textContent = t('users.short_password'); err.hidden = !bad; return !bad; },
      }],
    });
    if (!r) return;
    await withAdmin(null, (pw) => put(`/api/v1/users/${encodeURIComponent(u.name)}/password`, { password: pass.value, admin_password: pw }), t('common.saved'));
  };

  async function draw() {
    const res = await tryGet('/api/v1/users');
    if (failed(res)) { body.replaceChildren(errBox(res)); return; }
    const list = res.users || [];
    body.replaceChildren(panel(null, h('div.scroll', h('table.table', { 'data-testid': 'users' },
      h('thead', h('tr', [t('users.login'), t('users.full_name'), t('users.rights'), t('users.samba'), ''].map((x) => h('th', x)))),
      h('tbody', list.map((u) => h('tr', { 'data-user': u.name },
        h('td.mono', u.name), h('td', u.full_name || '—'),
        h('td', u.admin ? chip(t('users.admin'), 'ok') : u.nas ? chip(t('users.member')) : chip(t('users.no_nas'))),
        h('td', u.samba ? chip(t('users.samba_on'), 'ok') : chip(t('users.samba_off'))),
        h('td.actions',
          h('button.small.ghost', { type: 'button', onclick: () => setPassword(u) }, t('users.password')),
          h('button.small.ghost', { type: 'button', onclick: () => withAdmin(t(u.admin ? 'users.revoke_text' : 'users.grant_text', { name: u.name }), (pw) => put(`/api/v1/users/${encodeURIComponent(u.name)}/admin`, { admin: !u.admin, admin_password: pw })) }, t(u.admin ? 'users.revoke' : 'users.grant')),
          h('button.small.danger', { type: 'button', onclick: async () => {
            if (!(await confirmDialog({ title: t('users.delete'), text: t('users.delete_text', { name: u.name }), ok: t('users.delete'), danger: true }))) return;
            await withAdmin(null, (pw) => del(`/api/v1/users/${encodeURIComponent(u.name)}`, { admin_password: pw }));
          } }, t('users.delete'))))))))),
    h('p.caption', t('users.footer')));
  }
  await draw();
}

// --- sharing and SSH (DIST-018) ----------------------------------------------------------

async function sharing(main, { state }) {
  const body = h('div.stack');
  main.append(head(t('share.title'), t('share.subtitle')), body);
  const draw = async () => {
    const parts = [];
    const sh = await tryGet('/api/v1/sharing');
    if (failed(sh)) parts.push(errBox(sh));
    else {
      const set = (k) => (on) => put('/api/v1/sharing', { [k]: on }).then(() => toast(t('common.saved'), 'ok')).catch((e) => { fail(e); throw e; });
      parts.push(panel(t('share.folders'),
        h('div.stack',
          h('div.row.between', h('div', h('div', t('share.guest')), h('div.caption', t('share.guest_hint'))), switchEl({ checked: sh.media_guest_read, onChange: set('media_guest_read'), testid: 'share-guest' })),
          h('div.row.between', h('div', h('div', t('share.tm')), h('div.caption', t('share.tm_hint'))), switchEl({ checked: sh.time_machine, onChange: set('time_machine'), testid: 'share-tm' }))),
        h('hr'),
        kv((sh.shares || []).map((s) => [s.name, h('span', h('span.mono', `\\\\${state.system?.hostname || 'kinolune'}\\${s.name}`), s.comment ? h('span.caption', ` — ${s.comment}`) : null)])),
        h('p.caption', t('share.how'))));
    }
    if (state.features.ssh) {
      const ssh = await tryGet('/api/v1/ssh');
      if (failed(ssh)) parts.push(errBox(ssh));
      else {
        parts.push(panel('SSH',
          h('div.row.between', h('div', h('div', t('ssh.enabled')), h('div.caption', t('ssh.hint', { n: ssh.authorized_keys ?? 0 }))),
            switchEl({ checked: ssh.enabled, testid: 'ssh', onChange: async (on) => {
              if (on && !(await confirmDialog({ title: 'SSH', text: t('ssh.confirm'), ok: t('ssh.turn_on') }))) throw new Error('cancelled');
              await put('/api/v1/ssh', { enabled: on }).then(() => toast(t('common.saved'), 'ok')).catch((e) => { fail(e); throw e; });
            } })),
          h('div.row', ssh.active ? chip(t('svc.active'), 'ok') : chip(t('svc.inactive')), ssh.port_open ? chip(t('ssh.port_open'), 'warn') : chip(t('ssh.port_closed')))));
      }
    }
    body.replaceChildren(...parts);
  };
  await draw();
}

// --- disks -------------------------------------------------------------------------------

async function disks(main) {
  const body = h('div.stack');
  main.append(head(t('disk.title'), t('disk.subtitle')), body);
  const blockRow = (b) => h('tr',
    h('td.mono', b.device), h('td', bytes(b.size)), h('td', [b.fs_type, b.label].filter(Boolean).join(' · ') || b.usage || '—'),
    h('td.mono', (b.mount_points || []).join(', ') || '—'),
    h('td.actions',
      b.mountable && !(b.mount_points || []).length ? h('button.small', { type: 'button', 'data-testid': `mount-${b.device}`, onclick: async () => {
        try { const r = await post(`/api/v1/disks/${encodeURIComponent(b.device)}/mount`); toast(t('disk.mounted', { p: r?.mount_point || '' }), 'ok'); } catch (e) { fail(e); }
        draw().catch(fail);
      } }, t('disk.mount')) : null,
      b.unmountable && (b.mount_points || []).length ? h('button.small', { type: 'button', onclick: async () => {
        if (!(await confirmDialog({ title: t('disk.unmount'), text: t('disk.unmount_text', { d: b.device }), ok: t('disk.unmount') }))) return;
        try { await post(`/api/v1/disks/${encodeURIComponent(b.device)}/unmount`); toast(t('common.done'), 'ok'); } catch (e) { fail(e); }
        draw().catch(fail);
      } }, t('disk.unmount')) : null));
  const blocks = (list) => (list && list.length ? h('div.scroll', h('table.table',
    h('thead', h('tr', [t('disk.device'), t('disk.size'), t('disk.fs'), t('disk.mount_point'), ''].map((x) => h('th', x)))),
    h('tbody', list.map(blockRow)))) : null);
  async function draw() {
    const d = await tryGet('/api/v1/disks');
    if (failed(d)) { body.replaceChildren(errBox(d)); return; }
    const drives = (d.drives || []).map((x) => {
      const hl = x.health || {};
      return panel(null,
        h('div.row.between', h('div', h('h3', [x.vendor, x.model].filter(Boolean).join(' ') || x.id), h('div.caption', [bytes(x.size), x.bus, x.rotation_rpm ? `${x.rotation_rpm} rpm` : 'SSD', x.removable ? t('disk.removable') : null, x.system ? t('disk.system') : null].filter(Boolean).join(' · '))), healthChip(hl)),
        kv([
          [t('disk.serial'), x.serial],
          hl.temperature_c ? [t('disk.temp'), `${num(hl.temperature_c)} °C`] : null,
          hl.power_on_hours ? [t('disk.hours'), num(hl.power_on_hours)] : null,
          hl.smart_supported ? [t('disk.bad_sectors'), num(hl.bad_sectors || 0)] : [t('disk.smart'), t('disk.smart_off')],
        ]),
        (hl.problems || []).length ? h('div.alert.warn', h('ul', hl.problems.map((p) => h('li', p)))) : null,
        blocks(x.blocks));
    });
    body.replaceChildren(...(drives.length ? drives : [panel(null, h('p.muted', t('disk.none')))]), (d.other || []).length ? panel(t('disk.other'), blocks(d.other)) : null);
  }
  await draw();
}

// --- system: services, journal, power ----------------------------------------------------

async function system(main) {
  const body = h('div.stack');
  const powerBtn = (action) => h(`button${action === 'poweroff' ? '.danger' : ''}`, { type: 'button', 'data-testid': `power-${action}`, onclick: () => power(action) }, t(`power.${action}`));
  main.append(head(t('sys.title'), t('sys.subtitle'), powerBtn('reboot'), powerBtn('poweroff')), body);

  async function power(action) {
    // Step 1 without a token: the server answers 409 with a one-time token and the inhibitors.
    let first;
    try {
      await post(`/api/v1/power/${action}`);
      return waitReboot(main, t(`power.${action}_going`));
    } catch (e) {
      if (!(e instanceof ApiError) || e.body.error !== 'confirmation_required') return fail(e);
      first = e.body;
    }
    const inh = (first.inhibitors || []);
    const ok = await confirmDialog({
      title: t(`power.${action}`), text: t(`power.${action}_text`), ok: t(`power.${action}`), danger: true,
      extra: inh.length ? h('div.alert.warn', t('power.inhibitors'), h('ul', inh.map((i) => h('li', `${i.who}: ${i.why}`)))) : null,
    });
    if (!ok) return;
    try {
      await post(`/api/v1/power/${action}`, { confirm: first.confirm_token });
    } catch (e) { return fail(e); }
    if (action === 'reboot') await waitReboot(main, t('power.reboot_going'));
    else main.replaceChildren(head(t('power.poweroff')), panel(null, h('p', t('power.poweroff_done'))));
  }

  const journal = async (u) => {
    const box = h('div.journal', { tabindex: 0 }, t('common.loading'));
    dialog({ title: t('sys.journal', { name: u.title || u.name }), body: box, buttons: [{ label: t('common.close'), kind: 'primary', value: true }] });
    try {
      const j = await get(`/api/v1/services/${encodeURIComponent(u.name)}/journal?lines=200`);
      box.replaceChildren(...(j.entries || []).map((e) => h(`div.p${e.priority ?? 6}`, `${dateTime(e.time)}  ${e.message}`)));
      if (!(j.entries || []).length) box.textContent = t('sys.journal_empty');
      box.scrollTop = box.scrollHeight;
    } catch (e) { box.textContent = errText(e); }
  };

  async function draw() {
    const s = await tryGet('/api/v1/services');
    if (failed(s)) { body.replaceChildren(errBox(s)); return; }
    body.replaceChildren(panel(t('sys.services'), h('div.scroll', h('table.table', { 'data-testid': 'services' },
      h('thead', h('tr', [t('sys.service'), t('sys.state'), t('sys.since'), ''].map((x) => h('th', x)))),
      h('tbody', (s.services || []).map((u) => h('tr',
        h('td', h('div', u.title || u.name), h('div.caption.mono', u.name)),
        h('td', activeChip(u), u.restarts ? h('span.caption', ` ${t('sys.restarts', { n: u.restarts })}`) : null),
        h('td', dateTime(u.since)),
        h('td.actions',
          h('button.small.ghost', { type: 'button', onclick: () => journal(u) }, t('sys.log')),
          u.restartable ? h('button.small', { type: 'button', onclick: async () => {
            if (u.name === 'alpha-admin.service' && !(await confirmDialog({ title: t('sys.restart'), text: t('sys.restart_admin'), ok: t('sys.restart') }))) return;
            try { await post(`/api/v1/services/${encodeURIComponent(u.name)}/restart`); toast(t('sys.restarted', { name: u.title || u.name }), 'ok'); } catch (e) { fail(e); }
            setTimeout(() => draw().catch(fail), 1500);
          } }, t('sys.restart')) : null))))))));
  }
  await draw();
  every(10000, () => draw().catch(fail));
}

// --- advanced mode (Cockpit) -------------------------------------------------------------

async function advanced(main) {
  const body = h('div.stack');
  main.append(head(t('adv.title'), t('adv.subtitle')), body);
  const draw = async () => {
    const [fw, unit] = await Promise.all([tryGet('/api/v1/firewall'), tryGet('/api/v1/services/cockpit.socket')]);
    const opening = !failed(fw) && (fw.openings || []).find((o) => o.name === 'cockpit');
    const running = !failed(unit) && unit.active_state === 'active';
    const url = `https://${location.hostname}:9090/`;
    const noBlur = document.documentElement.classList.contains('no-blur');
    body.replaceChildren(
      panel('Cockpit',
        h('p', t('adv.cockpit_text')),
        h('div.alert.warn', t('adv.lan_only')),
        failed(fw) ? errBox(fw) : opening ? h('div.row.between', h('div', t('adv.open_port')), switchEl({ checked: opening.open, testid: 'fw-cockpit', onChange: (on) => put('/api/v1/firewall/openings/cockpit', { open: on }).then(() => { toast(t('common.saved'), 'ok'); draw().catch(fail); }).catch((e) => { fail(e); throw e; }) })) : null,
        h('div.row', failed(unit) ? null : running ? chip(t('adv.running'), 'ok') : chip(t('adv.stopped'))),
        !running && !failed(unit) ? h('p.caption', t('adv.not_running')) : null,
        h('div.row', h('a.btn.primary', { href: url, target: '_blank', rel: 'noopener noreferrer', 'data-testid': 'cockpit-open', 'aria-disabled': opening && opening.open && running ? null : 'true' }, t('adv.open')))),
      panel(t('adv.ui'),
        h('div.row.between', h('div', h('div', t('adv.no_blur')), h('div.caption', t('adv.no_blur_hint'))),
          switchEl({ checked: noBlur, testid: 'no-blur', onChange: async (on) => { document.documentElement.classList.toggle('no-blur', on); localStorage.setItem('kinolune-admin-no-blur', on ? '1' : '0'); } }))),
      panel(t('adv.api'), h('p.muted', t('adv.api_text')), h('a', { href: '/api/v1/openapi.yaml', target: '_blank' }, '/api/v1/openapi.yaml')));
  };
  await draw();
}

export const PAGES = {
  dashboard: { title: 'nav.dashboard', icon: 'home', render: dashboard },
  updates: { title: 'nav.updates', icon: 'update', render: updates },
  network: { title: 'nav.network', icon: 'network', render: network },
  users: { title: 'nav.users', icon: 'users', feature: 'users', render: users },
  sharing: { title: 'nav.sharing', icon: 'share', feature: 'sharing', render: sharing },
  disks: { title: 'nav.disks', icon: 'disk', render: disks },
  system: { title: 'nav.system', icon: 'system', render: system },
  advanced: { title: 'nav.advanced', icon: 'advanced', render: advanced },
};
