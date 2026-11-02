// 「進行中」頁面：每 3 秒向後端取一次資料；頁面不可見時暫停，回到可見時立即更新。
import { renameCommand } from './rename-utils.js';
import { cardKey } from './card-utils.js';
import { parsePins, prunePins, togglePin } from './pin-utils.js';
import { orderCards, parsePriorities, priorityOf, prunePriorities, setPriority } from './priority-utils.js';
import { contextLabel } from './context-utils.js';
import { CONFIRM_MS, closeNotice, isArmed, pressClose } from './close-utils.js';

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
// 回傳 { ok, data } 或 { ok, message }；錯誤內容不一定是 JSON，所以解析失敗時退回狀態碼。
async function post(path, item) {
  try {
    const response = await fetch(path, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ agent: item.agent, id: item.id }),
    });
    if (response.ok) return { ok: true, data: await response.json().catch(() => ({})) };
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
const PRIORITY_TAG = { high: '高', low: '低' };

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
function renderCard(session, pinned, priority) {
  const card = element('article', `live-card live-status-${session.status}${pinned ? ' live-pinned' : ''}`);
  card.dataset.key = cardKey(session);
  card.tabIndex = 0;
  card.setAttribute('role', 'button');
  const state = STATUS_LABEL[session.status] || session.status;
  card.title = `${state} · 跳到這個 session 的終端機分頁`;
  card.addEventListener('click', () => focusSession(session));
  card.addEventListener('keydown', (event) => {
    if (event.target !== card) return; // 卡片裡的按鈕自己處理
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      focusSession(session);
    }
  });
  const head = element('div', 'live-card-head');
  const side = element('span', 'live-card-side');
  // 閒置時只留給螢幕閱讀器，見 live.css
  side.append(element('span', 'live-card-state', `● ${state}`));
  if (session.agent === 'claude') side.append(renderRenameButton(session)); // Codex 沒有對應的 /rename
  side.append(renderPriorityButton(session, priority), renderPinButton(session, pinned));
  if (session.status !== 'working') side.append(renderCloseButton(session)); // 工作中被關會中斷正在跑的回合
  head.append(renderTitle(session.name || session.folder, priority), side);
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

function renderTitle(text, priority) {
  const title = element('span', 'live-card-title', text);
  const label = PRIORITY_TAG[priority];
  if (label) {
    const tag = element('span', 'live-priority-tag', label);
    tag.dataset.level = priority;
    tag.title = `優先度：${label}`;
    title.prepend(tag);
  }
  return title;
}

function renderMeta(session) {
  const meta = element('div', 'live-card-meta');
  const parts = [session.name ? session.folder : null, AGENT_TAG[session.agent]].filter(Boolean);
  if (session.context) {
    const { text, title } = contextLabel(session.context);
    const context = element('span', 'live-card-context', text);
    context.title = title;
    parts.push(context);
  }
  parts.forEach((part, index) => {
    if (index > 0) meta.append(' · ');
    meta.append(part);
  });
  return meta;
}

// 釘選與優先度存在這個瀏覽器，每次使用前重讀：兩個分頁同時開著時，才不會拿舊的資料互相蓋掉。
// localStorage 不能用（例如被瀏覽器擋掉）時只記在這次瀏覽
let storageUsable = true;
let lastData = null; // 切換釘選或優先度時立刻用上一份資料重畫，不等下一次輪詢

function localStore(key, parse, initial) {
  let value = initial;
  const load = () => {
    if (!storageUsable) return value;
    try {
      value = parse(localStorage.getItem(key));
    } catch {
      storageUsable = false;
    }
    return value;
  };
  // 內容沒變就不寫：每 3 秒清理一次，多半什麼都沒清掉
  const update = (change) => {
    const before = load();
    const after = change(before);
    if (JSON.stringify(after) === JSON.stringify(before)) return;
    value = after;
    if (!storageUsable) return;
    try {
      localStorage.setItem(key, JSON.stringify(after));
    } catch {
      storageUsable = false;
    }
  };
  return { load, update };
}

const pinStore = localStore('live-pins', parsePins, []);
const priorityStore = localStore('live-priorities', parsePriorities, {});

function renderPinButton(session, pinned) {
  const button = element('button', 'live-pin', '📌');
  button.type = 'button';
  button.dataset.key = `pin:${cardKey(session)}`;
  button.setAttribute('aria-pressed', String(pinned));
  button.setAttribute('aria-label', '釘選在最前面');
  button.title = pinned ? '取消釘選' : '釘選在最前面';
  button.addEventListener('click', (event) => {
    event.stopPropagation(); // 點卡片本身是跳轉
    pinStore.update((pins) => togglePin(pins, cardKey(session)));
    if (lastData) render(lastData);
  });
  return button;
}

// 優先度選單放在卡片外面：卡片每 3 秒整個重畫，放在卡片裡會被關掉。不做「點一下換下一級」：
// 標成高的卡片會馬上換位置，連點時第二下可能點到別張卡片
const priorityMenu = document.getElementById('priority-menu');
let priorityMenuKey = null;

function renderPriorityButton(session, priority) {
  const key = cardKey(session);
  const button = element('button', 'live-priority', '⚑');
  button.type = 'button';
  button.dataset.key = `priority:${key}`;
  button.dataset.level = priority;
  button.setAttribute('aria-haspopup', 'true');
  button.setAttribute('aria-expanded', String(priorityMenuKey === key));
  button.setAttribute('aria-label', '設定優先度');
  button.title = '設定優先度';
  button.addEventListener('click', (event) => {
    event.stopPropagation(); // 點卡片本身是跳轉
    if (priorityMenuKey === key) closePriorityMenu();
    else openPriorityMenu(key, priority, button);
  });
  return button;
}

function findPriorityButton(key) {
  return document.querySelector(`[data-key="${CSS.escape(`priority:${key}`)}"]`);
}

// 選單放在捲動的 .live-main 裡，以它的內容座標定位，捲動時才會跟著卡片走；下方放不下就翻到按鈕上方
function placePriorityMenu(button) {
  const area = priorityMenu.offsetParent;
  const box = area.getBoundingClientRect();
  const rect = button.getBoundingClientRect();
  const { offsetWidth: width, offsetHeight: height } = priorityMenu;
  const left = Math.max(8, Math.min(rect.right - box.left - width, area.clientWidth - width - 8));
  const below = rect.bottom + 4 + height <= box.bottom || rect.top - 4 - height < box.top;
  const top = below ? rect.bottom - box.top + 4 : rect.top - box.top - height - 4;
  priorityMenu.style.left = `${area.scrollLeft + left}px`;
  priorityMenu.style.top = `${area.scrollTop + top}px`;
}

// 開關選單都不重畫卡片：在點下去的當下換掉卡片，這一下就點不到卡片或它的按鈕了
function openPriorityMenu(key, priority, button) {
  findPriorityButton(priorityMenuKey)?.setAttribute('aria-expanded', 'false');
  priorityMenuKey = key;
  button.setAttribute('aria-expanded', 'true');
  showCurrentLevel(priority);
  priorityMenu.hidden = false;
  placePriorityMenu(button);
  priorityMenu.querySelector('[aria-pressed="true"]').focus();
}

function showCurrentLevel(priority) {
  for (const item of priorityMenu.querySelectorAll('button')) {
    item.setAttribute('aria-pressed', String(item.dataset.level === priority));
  }
}

function closePriorityMenu({ returnFocus = false } = {}) {
  const button = findPriorityButton(priorityMenuKey);
  priorityMenuKey = null;
  priorityMenu.hidden = true;
  button?.setAttribute('aria-expanded', 'false');
  if (returnFocus) button?.focus();
}

// 重畫或縮放視窗後，卡片可能換了位置、等級可能被另一個分頁改了；
// session 已經不在「進行中」就關掉，「最近結束」的優先度不能改
function syncPriorityMenu() {
  if (!priorityMenuKey) return;
  const button = findPriorityButton(priorityMenuKey);
  if (!button) return closePriorityMenu();
  placePriorityMenu(button);
  showCurrentLevel(button.dataset.level);
}
window.addEventListener('resize', syncPriorityMenu);

priorityMenu.addEventListener('click', (event) => {
  const item = event.target.closest('button');
  if (!item) return;
  const key = priorityMenuKey;
  priorityStore.update((priorities) => setPriority(priorities, key, item.dataset.level));
  closePriorityMenu({ returnFocus: true });
  render(lastData); // 換了優先度要重新排序；焦點跟著 ⚑ 走
});
// 掛在整份文件：用 Tab 離開選單後，Esc 一樣能關。改名對話框開著時 Esc 是關對話框
document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape' && priorityMenuKey && !renameDialog.open) closePriorityMenu({ returnFocus: true });
});
// ⚑ 自己處理開關
document.addEventListener('pointerdown', (event) => {
  if (priorityMenuKey && !priorityMenu.contains(event.target) && !event.target.closest('.live-priority')) {
    closePriorityMenu();
  }
});

// 關閉：結束 session 並關掉它的分頁，等同在 CLI 打 /exit 再關分頁。要點兩下才送出（見 close-utils.js），
// 每 3 秒重畫時按鈕會重建，所以「準備關閉」與「關閉中」都記在卡片外面
let closeArmed = null;
let closeDisarmTimer = null;
const closing = new Set();

function renderCloseButton(session) {
  const key = cardKey(session);
  const armed = isArmed(closeArmed, key, Date.now());
  const busy = closing.has(key);
  const label = busy ? '關閉中…' : armed ? '確定關閉？' : '✕';
  const button = element('button', `live-close${armed ? ' live-close-armed' : ''}`, label);
  button.type = 'button';
  button.dataset.key = `close:${key}`;
  button.disabled = busy;
  button.title = '結束這個 session 並關閉分頁';
  button.setAttribute('aria-label', armed ? '再按一次確定關閉' : button.title);
  button.addEventListener('click', (event) => {
    event.stopPropagation(); // 點卡片本身是跳轉
    const pressed = pressClose(closeArmed, key, Date.now());
    closeArmed = pressed.armed;
    clearTimeout(closeDisarmTimer);
    if (pressed.send) closeSession(session, key);
    else closeDisarmTimer = setTimeout(() => lastData && render(lastData), CONFIRM_MS);
    if (lastData) render(lastData);
  });
  return button;
}

async function closeSession(session, key) {
  closing.add(key);
  const result = await post('/api/live/close', session);
  closing.delete(key);
  notify(result.ok ? closeNotice(result.data) : result.message);
  refresh();
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
  button.dataset.key = `rename:${cardKey(session)}`;
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

// 優先度只顯示、不能改：標籤保留到接續，看得到才知道先接哪個
function renderRecent(item, priority) {
  const row = element('article', 'live-card live-recent');
  const head = element('div', 'live-card-head');
  head.append(renderTitle(recentTitle(item), priority));
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
  const pins = pinStore.load();
  const priorities = priorityStore.load();
  empty.hidden = data.live.length > 0;
  cards.hidden = data.live.length === 0;
  const focusedKey = document.activeElement?.dataset?.key; // 每 3 秒重畫，別讓鍵盤焦點跟著消失
  cards.replaceChildren(
    ...orderCards(data.live, pins, priorities).map((session) =>
      renderCard(session, pins.includes(cardKey(session)), priorityOf(priorities, session)),
    ),
  );
  recentEmpty.hidden = data.recent.length > 0;
  recentList.replaceChildren(...data.recent.map((item) => renderRecent(item, priorityOf(priorities, item))));
  if (focusedKey) document.querySelector(`[data-key="${CSS.escape(focusedKey)}"]`)?.focus();
  syncPriorityMenu();
  status.textContent = '';
}

async function refresh() {
  try {
    const response = await fetch('/api/live', { cache: 'no-store' });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const data = await response.json();
    // 只在成功取得時清理，連線失敗不會動到釘選與優先度
    pinStore.update((pins) => prunePins(pins, data.live, data.recent));
    priorityStore.update((priorities) => prunePriorities(priorities, data.live, data.recent));
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
