// Per-chat composer drafts in localStorage (every access guarded: storage may be unavailable).

const PREFIX = "wa-desk.draft.";

export function loadDraft(jid: string): string {
  try {
    return localStorage.getItem(PREFIX + jid) ?? "";
  } catch {
    return "";
  }
}

/** Save a draft; empty / whitespace-only text removes it. */
export function saveDraft(jid: string, text: string): void {
  try {
    if (text.trim()) localStorage.setItem(PREFIX + jid, text);
    else localStorage.removeItem(PREFIX + jid);
  } catch {
    /* storage unavailable or full */
  }
}

export function clearDraft(jid: string): void {
  saveDraft(jid, "");
}

export function hasDraft(jid: string): boolean {
  return loadDraft(jid).length > 0;
}
