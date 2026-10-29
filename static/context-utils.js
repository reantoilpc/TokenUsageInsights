// 999,500 起四捨五入成千位會變成「1000K」，所以從這裡就改用 M。
function tokenSize(tokens) {
  if (tokens >= 999_500) return `${(tokens / 1_000_000).toFixed(1)}M`;
  if (tokens >= 1000) return `${Math.round(tokens / 1000)}K`;
  return String(tokens);
}

// 剛壓縮時不寫數字：壓縮記下的大小比下一次實際送出的少很多，壓縮前的大小又已經過時
export function contextLabel({ tokens, justCompacted }) {
  if (justCompacted) {
    return { text: 'context 剛壓縮', title: `壓縮前 ${tokenSize(tokens)}，下一次回覆後顯示新的大小` };
  }
  return { text: `context ${tokenSize(tokens)}`, title: '最後一次送出的 context 大小' };
}
