// 「進行中」頁面：每 3 秒向後端取一次資料；頁面不可見時暫停，回到可見時立即更新。
import { renameCommand } from './rename-utils.js';
import { orderByPins, parsePins, pinKey, prunePins, togglePin } from './pin-utils.js';
import { contextLabel } from './context-utils.js';

const POLL_MS = 3000;
let timer = null;

const cards = document.getElementById('live-cards');
const recentList = document.getElementById('recent-list');
const recentEmpty = document.getElementById('recent-empty');
const empty = document.getElementById('live-empty');
const status = document.getElementById('live-status');
const notice = document.getElementById('live-notice');
let noticeTimer = null;

function notify(text) {
  notice.textContent = text;
  clearTimeout(noticeTimer);
  noticeTimer = setTimeout(() => (notice.textContent = ''), 4000);
}

// 只送 agent 與 id；資料夾、名稱與指令一律由伺服器自己決定。
// 回傳 { ok, message }；錯誤內容不一定是 JSON，所以解析失敗時退回狀態碼。
async function post(path, item) {
  try {
    const response = await fetch(path, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ agent: item.agent, id: item.id }),
    });
    if (response.ok) return { ok: true };
    let message = '';
    try {
      message = (await response.json()).message;
    } catch {
      // 非 JSON 的錯誤內容
    }
    return { ok: false, message: message || `請求失敗（HTTP ${response.status}）` };
  } catch {
    return { ok: false, message: '無法連線到看板服務' };
  }
}

async function focusSession(session) {
  const result = await post('/api/live/focus', session);
  if (!result.ok) notify(result.message);
}

// 幾乎都是 Claude Code，每張都寫就成了雜訊：只標出 Codex
const AGENT_TAG = { codex: 'Codex' };
const STATUS_LABEL = { waiting: '等你回覆', working: '工作中', idle: '閒置' };

function element(tag, className, text) {
  const node = document.createElement(tag);
  node.className = className;
  if (text) node.textContent = text;
  return node;
}

function ago(ms) {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  if (seconds < 60) return `${seconds}s 前`;
  const minutes = Math.floor(seconds / 60);
  return minutes < 60 ? `${minutes}m 前` : `${Math.floor(minutes / 60)}h 前`;
}

// 一律用 textContent：名稱與資料夾來自對話紀錄，視為不可信輸入
function renderCard(session, pinned) {
  const card = element('article', `live-card live-status-${session.status}${pinned ? ' live-pinned' : ''}`);
  card.dataset.key = `${session.agent}:${session.id}`;
  card.tabIndex = 0;
  card.setAttribute('role', 'button');
  const state = STATUS_LABEL[session.status] || session.status;
  card.title = `${state} · 跳到這個 session 的終端機分頁`;
  card.addEventListener('click', () => focusSession(session));
  card.addEventListener('keydown', (event) => {
    if (event.target !== card) return; // 卡片裡的 ✎、📌 按鈕自己處理
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      focusSession(session);
    }
  });
  const head = element('div', 'live-card-head');
  const side = element('span', 'live-card-side');
  // 閒置時這段字只留給螢幕閱讀器（見 live.css），畫面上靠左邊框的顏色表示
  side.append(element('span', 'live-card-state', `● ${state}`));
  if (session.agent === 'claude') side.append(renderRenameButton(session)); // Codex 沒有對應的 /rename
  side.append(renderPinButton(session, pinned));
  head.append(element('span', 'live-card-title', session.name || session.folder), side);
  card.append(head);
  const meta = renderMeta(session);
  if (meta.hasChildNodes()) card.append(meta);
  if (session.progress) card.append(renderProgress(session.progress));
  // Claude Code 在我離開時寫的 recap：來自對話紀錄，一樣只用 textContent
  if (session.recap) card.append(element('div', 'live-card-recap', session.recap));
  if (session.lastPrompt) {
    // 最後一行：我最後下的指令，時間只有一個、無標籤、靠最右
    const foot = element('div', 'live-card-foot');
    foot.append(element('span', 'live-card-prompt', session.lastPrompt));
    if (session.promptAt) foot.append(element('span', 'live-card-time', ago(Date.now() - session.promptAt)));
    card.append(foot);
  }
  return card;
}

function renderMeta(session) {
  const meta = element('div', 'live-card-meta');
  const parts = [session.name ? session.folder : null, AGENT_TAG[session.agent]].filter(Boolean);
  if (session.context) {
    const context = element('span', 'live-card-context', contextLabel(session.context));
    context.title = '最後一次送出的 context 大小';
    parts.push(context);
  }
  parts.forEach((part, index) => {
    if (index > 0) meta.append(' · ');
    meta.append(part);
  });
  return meta;
}

// 釘選存在這個瀏覽器，每次使用前重讀：兩個分頁同時開著時，才不會拿舊的清單互相蓋掉
const PINS_KEY = 'live-pins';
let pins = [];
let storageUsable = true;
let lastData = null; // 切換釘選時立刻用上一份資料重畫，不等下一次輪詢

function loadPins() {
  if (!storageUsable) return pins;
  try {
    pins = parsePins(localStorage.getItem(PINS_KEY));
  } catch {
    storageUsable = false;
  }
  return pins;
}

function savePins(next) {
  pins = next;
  if (!storageUsable) return;
  try {
    localStorage.setItem(PINS_KEY, JSON.stringify(next));
  } catch {
    storageUsable = false;
  }
}

function renderPinButton(session, pinned) {
  const button = element('button', 'live-pin', '📌');
  button.type = 'button';
  button.dataset.key = `pin:${pinKey(session)}`;
  button.setAttribute('aria-pressed', String(pinned));
  button.setAttribute('aria-label', '釘選在最前面');
  button.title = pinned ? '取消釘選' : '釘選在最前面';
  button.addEventListener('click', (event) => {
    event.stopPropagation(); // 點卡片本身是跳轉
    savePins(togglePin(loadPins(), pinKey(session)));
    if (lastData) render(lastData);
  });
  return button;
}

// 改名是半自動：網頁只複製 /rename 指令並跳到分頁，由我自己在分頁貼上、按 Enter。
// 只在閒置時開放：等你回覆時貼上會打進問題的回答欄，工作中可能開著權限框（Enter 等於核准）。
const RENAME_BLOCKED = { working: '工作中，等它閒置再改名', waiting: '正在等你回覆，回覆後再改名' };
const renameDialog = document.getElementById('rename-dialog');
const renameForm = document.getElementById('rename-form');
const renameInput = document.getElementById('rename-input');
const renameError = document.getElementById('rename-error');
const renameManual = document.getElementById('rename-manual');
const renameCommandField = document.getElementById('rename-command');
const renameSubmit = document.getElementById('rename-submit');
// 對話框在卡片外面，每 3 秒重畫卡片不會清掉正在輸入的名稱
let renaming = null;

function renderRenameButton(session) {
  const button = element('button', 'live-rename', '✎');
  button.type = 'button';
  button.dataset.key = `rename:${session.agent}:${session.id}`;
  const blocked = session.status === 'idle' ? null : RENAME_BLOCKED[session.status] || '只有閒置時可以改名';
  // 用 aria-disabled 而不是 disabled：停用的按鈕不會顯示提示，點了也不會有反應
  if (blocked) button.setAttribute('aria-disabled', 'true');
  button.title = blocked || '改名（等同在 CLI 打 /rename）';
  button.setAttribute('aria-label', button.title);
  button.addEventListener('click', (event) => {
    event.stopPropagation(); // 點卡片本身是跳轉
    if (blocked) notify(blocked);
    else openRename(session);
  });
  return button;
}

function showManualCopy(visible, command = '') {
  renameManual.hidden = !visible;
  renameCommandField.value = command;
  renameSubmit.textContent = visible ? '前往分頁' : '複製並前往分頁';
}

function openRename(session) {
  renaming = session;
  renameInput.value = session.name || '';
  renameError.textContent = '';
  showManualCopy(false);
  renameDialog.showModal();
  renameInput.select();
}

async function goToTab(session, command) {
  renameDialog.close();
  const result = await post('/api/live/focus', session);
  // 跳不過去也已經複製好了，自己切過去貼上即可
  notify(result.ok
    ? `已複製「${command}」，請在分頁按 Ctrl+V、Enter`
    : `已複製「${command}」，但${result.message}，請自己切過去貼上`);
}

renameInput.addEventListener('input', () => {
  renameError.textContent = '';
  showManualCopy(false); // 名稱改了，先前顯示的指令就過時了
});
document.getElementById('rename-cancel').addEventListener('click', () => renameDialog.close());
renameForm.addEventListener('submit', async (event) => {
  event.preventDefault(); // 名稱不合格時對話框要留著
  if (!renameManual.hidden) return goToTab(renaming, renameCommandField.value);
  const result = renameCommand(renameInput.value);
  if (result.error) {
    renameError.textContent = result.error;
    return;
  }
  try {
    await navigator.clipboard.writeText(result.command);
  } catch {
    // 瀏覽器不給寫剪貼簿：顯示指令讓我自己複製，複製完再按「前往分頁」
    showManualCopy(true, result.command);
    renameCommandField.select();
    return;
  }
  goToTab(renaming, result.command);
});

// 剛按過「接續」的 session：每 3 秒重畫時按鈕會重建，停用狀態要跨重畫保留，否則會連點開出兩個分頁
const resuming = new Set();

async function resumeSession(item) {
  if (resuming.has(item.id)) return;
  resuming.add(item.id);
  document.querySelector(`[data-key="resume:${CSS.escape(item.id)}"]`)?.setAttribute('disabled', '');
  const result = await post('/api/live/resume', item);
  notify(result.ok ? '已開啟接續的分頁' : result.message);
  setTimeout(() => resuming.delete(item.id), 5000);
}

// 名稱規則：自訂標題；沒有就取最後指令的開頭；再沒有就用資料夾名稱
function recentTitle(item) {
  return item.name || (item.lastPrompt ? item.lastPrompt.slice(0, 30) : item.folder);
}

function renderRecent(item) {
  const row = element('article', 'live-card live-recent');
  const head = element('div', 'live-card-head');
  head.append(element('span', 'live-card-title', recentTitle(item)));
  const foot = element('div', 'live-card-foot');
  if (item.lastPrompt) foot.append(element('span', 'live-card-prompt', item.lastPrompt));
  foot.append(element('span', 'live-card-time', ago(Date.now() - item.updatedAt)));
  const resume = element('button', 'live-resume', '接續');
  resume.type = 'button';
  resume.dataset.key = `resume:${item.id}`;
  resume.disabled = resuming.has(item.id);
  resume.addEventListener('click', () => resumeSession(item));
  row.append(
    head,
    element('div', 'live-card-meta', [item.folder, AGENT_TAG[item.agent]].filter(Boolean).join(' · ')),
    foot,
    resume,
  );
  return row;
}

function renderProgress({ done, total, current }) {
  const bar = element('div', 'live-bar');
  const fill = element('span', 'live-bar-fill');
  fill.style.width = `${Math.round((done / total) * 100)}%`;
  bar.append(fill);
  const row = element('div', 'live-card-progress');
  row.append(bar, element('span', 'live-card-progress-text', current ? `${done}/${total} ${current}` : `${done}/${total}`));
  return row;
}

function render(data) {
  lastData = data;
  const current = loadPins();
  empty.hidden = data.live.length > 0;
  cards.hidden = data.live.length === 0;
  const focusedKey = document.activeElement?.dataset?.key; // 每 3 秒重畫，別讓鍵盤焦點跟著消失
  cards.replaceChildren(
    ...orderByPins(data.live, current).map((session) => renderCard(session, current.includes(pinKey(session)))),
  );
  recentEmpty.hidden = data.recent.length > 0;
  recentList.replaceChildren(...data.recent.map(renderRecent));
  if (focusedKey) document.querySelector(`[data-key="${CSS.escape(focusedKey)}"]`)?.focus();
  status.textContent = '';
}

async function refresh() {
  try {
    const response = await fetch('/api/live', { cache: 'no-store' });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const data = await response.json();
    // 只在成功取得時清理，連線失敗不會動到釘選
    const stored = loadPins();
    const kept = prunePins(stored, data.live, data.recent);
    if (kept.length !== stored.length) savePins(kept);
    render(data);
  } catch {
    status.textContent = '無法取得資料，稍後重試';
  }
}

function start() {
  if (timer !== null) return;
  refresh();
  timer = setInterval(refresh, POLL_MS);
}

function stop() {
  clearInterval(timer);
  timer = null;
}

document.addEventListener('visibilitychange', () => (document.hidden ? stop() : start()));
if (!document.hidden) start();
