// 「進行中」卡片與收藏上的相對時間。
export function ago(ms) {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  if (seconds < 60) return `${seconds}s 前`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m 前`;
  const hours = Math.floor(minutes / 60);
  return hours < 24 ? `${hours}h 前` : `${Math.floor(hours / 24)}d 前`; // 收藏常是幾天前結束的
}
