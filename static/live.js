// 「進行中」頁面：每 3 秒向後端取一次資料；頁面不可見時暫停，回到可見時立即更新。
const POLL_MS = 3000;
let timer = null;

const cards = document.getElementById('live-cards');
const empty = document.getElementById('live-empty');
const status = document.getElementById('live-status');
const notice = document.getElementById('live-notice');
let noticeTimer = null;

function notify(text) {
  notice.textContent = text;
  clearTimeout(noticeTimer);
  noticeTimer = setTimeout(() => (notice.textContent = ''), 4000);
}

// 只送 agent 與 id；資料夾與名稱由伺服器自己查
async function focusSession(session) {
  try {
    const response = await fetch('/api/live/focus', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ agent: session.agent, id: session.id }),
    });
    if (!response.ok) notify((await response.json()).message || '無法跳轉');
  } catch {
    notify('無法連線到看板服務');
  }
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
  card.title = '跳到這個 session 的 Ghostty 分頁';
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
  if (session.lastPrompt) {
    // 最後一行：我最後下的指令，時間只有一個、無標籤、靠最右
    const foot = element('div', 'live-card-foot');
    foot.append(element('span', 'live-card-prompt', session.lastPrompt));
    if (session.promptAt) foot.append(element('span', 'live-card-time', ago(Date.now() - session.promptAt)));
    card.append(foot);
  }
  return card;
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
  if (focusedKey) cards.querySelector(`[data-key="${CSS.escape(focusedKey)}"]`)?.focus();
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
