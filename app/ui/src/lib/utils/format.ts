// Formatting helpers. All timestamps are Unix seconds (UTC) per the contract;
// display happens in the local time zone.

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const MONTHS_LONG = [
  "January", "February", "March", "April", "May", "June",
  "July", "August", "September", "October", "November", "December",
];
const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const WEEKDAYS_LONG = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

function pad2(n: number): string {
  return n < 10 ? `0${n}` : String(n);
}

/** Local midnight (ms) of the day containing `ms`. */
function startOfDay(ms: number): number {
  const d = new Date(ms);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

/** Whole calendar days between two instants in local time (b - a). */
export function dayDiff(aMs: number, bMs: number): number {
  return Math.round((startOfDay(bMs) - startOfDay(aMs)) / 86_400_000);
}

/** "14:05" */
export function timeOfDay(ts: number): string {
  const d = new Date(ts * 1000);
  return `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

/** "12 Mar 2024" (year omitted when it matches `now`'s year unless `forceYear`). */
export function shortDate(ts: number, nowMs: number = Date.now(), forceYear = false): string {
  const d = new Date(ts * 1000);
  const sameYear = d.getFullYear() === new Date(nowMs).getFullYear();
  const base = `${d.getDate()} ${MONTHS[d.getMonth()]}`;
  return sameYear && !forceYear ? base : `${base} ${d.getFullYear()}`;
}

/** "12 Mar 2024, 14:05" */
export function dateTime(ts: number | null | undefined): string {
  if (ts == null) return "—";
  return `${shortDate(ts, Date.now(), true)}, ${timeOfDay(ts)}`;
}

/**
 * Compact relative time for lists: "now", "5m", "3h" (same day), "Yesterday",
 * weekday within a week, then "12 Mar" / "12 Mar 2023".
 */
export function relativeTime(ts: number | null | undefined, nowMs: number = Date.now()): string {
  if (ts == null) return "";
  const ms = ts * 1000;
  const diffS = Math.floor((nowMs - ms) / 1000);
  if (diffS < 0) return timeOfDay(ts);
  if (diffS < 60) return "now";
  const days = dayDiff(ms, nowMs);
  if (diffS < 3600) return `${Math.floor(diffS / 60)}m`;
  if (days === 0) return diffS < 6 * 3600 ? `${Math.floor(diffS / 3600)}h` : timeOfDay(ts);
  if (days === 1) return "Yesterday";
  if (days < 7) return WEEKDAYS[new Date(ms).getDay()] ?? "";
  return shortDate(ts, nowMs);
}

/** Verbose "time ago" for dashboards: "just now", "5 minutes ago", "3 days ago". */
export function timeAgo(ts: number | null | undefined, nowMs: number = Date.now()): string {
  if (ts == null) return "never";
  const s = Math.max(0, Math.floor(nowMs / 1000 - ts));
  if (s < 45) return "just now";
  const units: [number, string][] = [
    [31_536_000, "year"], [2_592_000, "month"], [86_400, "day"], [3600, "hour"], [60, "minute"],
  ];
  for (const [secs, name] of units) {
    if (s >= secs) {
      const n = Math.floor(s / secs);
      return `${n} ${name}${n === 1 ? "" : "s"} ago`;
    }
  }
  return "just now";
}

/** Day separator label: "Today", "Yesterday", "Monday", "Monday, 3 March 2025". */
export function dayLabel(ts: number, nowMs: number = Date.now()): string {
  const d = new Date(ts * 1000);
  const days = dayDiff(d.getTime(), nowMs);
  if (days === 0) return "Today";
  if (days === 1) return "Yesterday";
  const wd = WEEKDAYS_LONG[d.getDay()] ?? "";
  if (days > 1 && days < 7) return wd;
  return `${wd}, ${d.getDate()} ${MONTHS_LONG[d.getMonth()]} ${d.getFullYear()}`;
}

/** Stable key for "same local day" comparisons. */
export function dayKey(ts: number): string {
  const d = new Date(ts * 1000);
  return `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;
}

/** Human file size with binary units: "0 B", "999 B", "1.5 KB", "12 MB", "1.07 GB". */
export function formatSize(bytes: number | null | undefined): string {
  if (bytes == null || !Number.isFinite(bytes) || bytes < 0) return "—";
  if (bytes < 1024) return `${Math.round(bytes)} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = bytes / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  const digits = v >= 100 ? 0 : v >= 10 ? 1 : 2;
  const s = v.toFixed(digits).replace(/\.0+$/, "").replace(/(\.\d*[1-9])0+$/, "$1");
  return `${s} ${units[i]}`;
}

/** "1:05", "1:02:03" */
export function formatClock(secs: number): string {
  if (!Number.isFinite(secs) || secs < 0) return "0:00";
  const s = Math.floor(secs % 60);
  const m = Math.floor((secs / 60) % 60);
  const h = Math.floor(secs / 3600);
  return h > 0 ? `${h}:${pad2(m)}:${pad2(s)}` : `${m}:${pad2(s)}`;
}

/** "850 ms", "12 s", "3 min 4 s", "2 h 5 min" */
export function formatDuration(secs: number | null | undefined): string {
  if (secs == null || !Number.isFinite(secs) || secs < 0) return "—";
  if (secs < 1) return `${Math.round(secs * 1000)} ms`;
  if (secs < 60) return `${Math.round(secs)} s`;
  if (secs < 3600) {
    const m = Math.floor(secs / 60);
    const s = Math.round(secs % 60);
    return s ? `${m} min ${s} s` : `${m} min`;
  }
  const h = Math.floor(secs / 3600);
  const m = Math.round((secs % 3600) / 60);
  return m ? `${h} h ${m} min` : `${h} h`;
}

/** Thousands separators: 12345 → "12,345". */
export function formatCount(n: number): string {
  return Math.round(n).toString().replace(/\B(?=(\d{3})+(?!\d))/g, ",");
}

/** Known country codes (longest match first) for phone grouping. */
const COUNTRY_CODES = ["971", "880", "91", "44", "49", "33", "61", "65", "86", "81", "7", "1"];

/**
 * Display a JID or bare phone number nicely.
 * "919876543210@s.whatsapp.net" → "+91 98765 43210"; "1234@lid" → "~1234";
 * groups ("…@g.us") and unknown shapes are returned unchanged.
 */
export function formatPhone(jidOrPhone: string | null | undefined): string {
  if (!jidOrPhone) return "";
  const [user = "", server = ""] = jidOrPhone.split("@");
  if (server === "lid") return `~${user}`;
  if (server && server !== "s.whatsapp.net" && server !== "c.us") return jidOrPhone;
  const digits = user.replace(/[^\d]/g, "");
  if (digits.length < 7 || digits.length > 15 || digits.length !== user.replace(/^\+/, "").length) {
    return jidOrPhone;
  }
  const cc = COUNTRY_CODES.find((c) => digits.startsWith(c) && digits.length - c.length >= 6) ?? "";
  const rest = digits.slice(cc.length);
  let grouped: string;
  if (cc === "91" && rest.length === 10) grouped = `${rest.slice(0, 5)} ${rest.slice(5)}`;
  else if (cc === "1" && rest.length === 10) grouped = `${rest.slice(0, 3)} ${rest.slice(3, 6)} ${rest.slice(6)}`;
  else grouped = rest.replace(/(\d{3,4})(?=(\d{3,4})+$)/g, "$1 ").trim();
  return cc ? `+${cc} ${grouped}` : `+${grouped}`;
}

/** True when the string looks like a raw JID / number rather than a resolved name. */
export function looksLikeNumber(name: string): boolean {
  return /^[+~]?[\d\s-]{6,}$/.test(name) || name.includes("@");
}

/**
 * A sender label: resolved name, or a nicely formatted phone.
 * LID senders show "~Push name" when a name is known, else "Unknown contact".
 */
export function displayName(name: string, jid?: string | null): string {
  const isLid = !!jid && jid.endsWith("@lid");
  if (name && !looksLikeNumber(name)) return isLid && !name.startsWith("~") ? `~${name}` : name;
  if (isLid || name.endsWith("@lid")) return "Unknown contact";
  if (jid) return formatPhone(jid);
  return name.includes("@") ? formatPhone(name) : name;
}

/** Chat title: never a raw number/JID when a better label exists. */
export function chatLabel(c: { name: string; jid: string; chat_type?: string | undefined }): string {
  if (c.name && !looksLikeNumber(c.name)) return c.name;
  const t = c.chat_type ?? (c.jid.endsWith("@g.us") ? "group" : "direct");
  if (t === "group") return "Unnamed group";
  if (t === "community") return "Unnamed community";
  if (t === "channel") return "Unnamed channel";
  if (t === "broadcast") return "Broadcast list";
  if (c.jid.endsWith("@lid")) return "Unknown contact";
  return formatPhone(c.jid) || c.name;
}

/** Short type label for subtitles. */
export function chatTypeLabel(t: string): string {
  return { direct: "Contact", group: "Group", community: "Community", channel: "Channel", broadcast: "Broadcast list", status: "Status" }[t] ?? t;
}

/** "HH:MM" validity (00:00–23:59). */
export function isHHMM(s: string): boolean {
  return /^([01]\d|2[0-3]):[0-5]\d$/.test(s);
}

/** Percentage 0..100 (guards divide-by-zero). */
export function pct(done: number, total: number): number {
  if (!total || total <= 0) return 0;
  return Math.max(0, Math.min(100, (done / total) * 100));
}

/** Local date → "YYYY-MM-DD" for <input type=date>. */
export function toDateInput(ts: number | null): string {
  if (ts == null) return "";
  const d = new Date(ts * 1000);
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
}

/** "YYYY-MM-DD" → Unix seconds at local start (or end) of that day. */
export function fromDateInput(s: string, endOfDay = false): number | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(s);
  if (!m) return null;
  const d = new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]));
  if (endOfDay) d.setHours(23, 59, 59, 0);
  return Math.floor(d.getTime() / 1000);
}

/** Emoji used to label a media kind in previews. */
export function mediaEmoji(kind: "image" | "video" | "audio" | "document" | "sticker"): string {
  return { image: "📷", video: "🎥", audio: "🎤", document: "📄", sticker: "💟" }[kind];
}
