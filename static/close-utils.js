// 「進行中」卡片的關閉鈕：第一下只是準備，3 秒內在同一張卡片再點一次才真的送出。
// armed 是 { key, at } 或 null：準備要關的卡片，以及按下第一下的時間。

export const CONFIRM_MS = 3000;

export function isArmed(armed, key, now) {
  return Boolean(armed) && armed.key === key && now - armed.at < CONFIRM_MS;
}

// 回傳按下之後的狀態，以及這一下要不要送出
export function pressClose(armed, key, now) {
  if (isArmed(armed, key, now)) return { armed: null, send: true };
  return { armed: { key, at: now }, send: false };
}

// 伺服器回報的結果（{ ended, tabClosed }）要怎麼告訴我
export function closeNotice({ ended, tabClosed }) {
  if (!ended) return '已送出結束訊號，程序還沒結束';
  if (!tabClosed) return '已結束，但找不到它的分頁，分頁沒有關';
  return '已關閉 session 與分頁';
}
