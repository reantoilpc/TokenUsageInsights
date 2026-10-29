// 999,500 起四捨五入成千位會變成「1000K」，所以從這裡就改用 M。
export function contextLabel(tokens) {
  if (tokens >= 999_500) return `context ${(tokens / 1_000_000).toFixed(1)}M`;
  if (tokens >= 1000) return `context ${Math.round(tokens / 1000)}K`;
  return `context ${tokens}`;
}
