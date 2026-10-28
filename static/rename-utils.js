// 「進行中」卡片的改名：產生要貼進 Claude Code 的 /rename 指令。

// Claude Code 的 session 名稱上限
const MAX_LENGTH = 200;
// 控制字元（含換行、Tab）、格式字元（零寬空白、方向控制等看不見的字）、行與段落分隔符
const HIDDEN_CHARACTERS = /[\p{Cc}\p{Cf}\p{Zl}\p{Zp}]/u;

export function renameCommand(name) {
  const trimmed = name.trim();
  if (!trimmed) return { error: '請輸入名稱' };
  if ([...trimmed].length > MAX_LENGTH) return { error: `名稱最多 ${MAX_LENGTH} 字` };
  // 貼上後 Enter 會變成換行或只清掉隱形字元，指令就會卡在輸入框裡
  if (HIDDEN_CHARACTERS.test(trimmed)) return { error: '名稱不能有換行、控制字元或看不見的字元' };
  // Claude Code 把結尾的反斜線加 Enter 當成換行，指令會送不出去
  if (trimmed.endsWith('\\')) return { error: '名稱不能以反斜線結尾' };
  return { command: `/rename ${trimmed}` };
}
