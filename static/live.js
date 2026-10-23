// 「進行中」頁面：每 3 秒向後端取一次資料；頁面不可見時暫停，回到可見時立即更新。
const POLL_MS = 3000;
let timer = null;

const cards = document.getElementById('live-cards');
const empty = document.getElementById('live-empty');
const status = document.getElementById('live-status');

const AGENT_LABEL = { claude: 'Claude Code', codex: 'Codex' };
const STATUS_LABEL = { waiting: '等你回覆', working: '工作中', idle: '閒置' };

function element(tag, className, text) {
  const node = document.createElement(tag);
  node.className = className;
  if (text) node.textContent = text;
  return node;
}

// 一律用 textContent：名稱與資料夾來自對話紀錄，視為不可信輸入
function renderCard(session) {
  const card = element('article', `live-card live-status-${session.status}`);
  const head = element('div', 'live-card-head');
  head.append(
    element('span', 'live-card-title', session.name || session.folder),
    element('span', 'live-card-state', `● ${STATUS_LABEL[session.status] || session.status}`),
  );
  const meta = [session.name ? session.folder : null, AGENT_LABEL[session.agent] || session.agent];
  card.append(head, element('div', 'live-card-meta', meta.filter(Boolean).join(' · ')));
  return card;
}

function render(data) {
  empty.hidden = data.live.length > 0;
  cards.hidden = data.live.length === 0;
  cards.replaceChildren(...data.live.map(renderCard));
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
