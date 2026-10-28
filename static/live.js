// 「進行中」頁面：每 3 秒向後端取一次資料；頁面不可見時暫停，回到可見時立即更新。
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

const AGENT_LABEL = { claude: 'Claude Code', codex: 'Codex' };
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
function renderCard(session) {
  const card = element('article', `live-card live-status-${session.status}`);
  card.dataset.key = `${session.agent}:${session.id}`;
  card.tabIndex = 0;
  card.setAttribute('role', 'button');
  card.title = '跳到這個 session 的終端機分頁';
  card.addEventListener('click', () => focusSession(session));
  card.addEventListener('keydown', (event) => {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      focusSession(session);
    }
  });
  const head = element('div', 'live-card-head');
  head.append(
    element('span', 'live-card-title', session.name || session.folder),
    element('span', 'live-card-state', `● ${STATUS_LABEL[session.status] || session.status}`),
  );
  const meta = [session.name ? session.folder : null, AGENT_LABEL[session.agent] || session.agent];
  card.append(head, element('div', 'live-card-meta', meta.filter(Boolean).join(' · ')));
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
    element('div', 'live-card-meta', `${item.folder} · ${AGENT_LABEL[item.agent] || item.agent}`),
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
  empty.hidden = data.live.length > 0;
  cards.hidden = data.live.length === 0;
  const focusedKey = document.activeElement?.dataset?.key; // 每 3 秒重畫，別讓鍵盤焦點跟著消失
  cards.replaceChildren(...data.live.map(renderCard));
  recentEmpty.hidden = data.recent.length > 0;
  recentList.replaceChildren(...data.recent.map(renderRecent));
  if (focusedKey) document.querySelector(`[data-key="${CSS.escape(focusedKey)}"]`)?.focus();
  status.textContent = '';
}

async function refresh() {
  try {
    const response = await fetch('/api/live', { cache: 'no-store' });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    render(await response.json());
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
