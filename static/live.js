// 「進行中」頁面：每 3 秒向後端取一次資料；頁面不可見時暫停，回到可見時立即更新。
const POLL_MS = 3000;
let timer = null;

const cards = document.getElementById('live-cards');
const empty = document.getElementById('live-empty');
const status = document.getElementById('live-status');

function render(data) {
  empty.hidden = data.live.length > 0;
  cards.hidden = data.live.length === 0;
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
