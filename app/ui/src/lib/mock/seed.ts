// Deterministic seed data for the mock backend: ~40 chats, ~3000 messages.

import type { AutomationEvent } from "../bindings/AutomationEvent";
import type { Issue } from "../bindings/Issue";
import type { JobRecord } from "../bindings/JobRecord";
import type { LogLevel } from "../bindings/LogLevel";
import type { LogLine } from "../bindings/LogLine";
import type { MediaKind } from "../bindings/MediaKind";
import type { MediaRef } from "../bindings/MediaRef";
import type { Message } from "../bindings/Message";
import type { Rule } from "../bindings/Rule";
import { hashString, mulberry32 } from "../utils/misc";
import { mediaEmoji } from "../utils/format";
import {
  MOCK_AUDIO_URL, MOCK_PDF_URL, MOCK_VIDEO_URL, pdfThumbSvg, photoSvg, stickerSvg, textDataUrl, videoPosterSvg,
} from "./assets";

export interface Contact {
  name: string;
  phone: string;
}

export interface MockChat {
  jid: string;
  name: string;
  is_group: boolean;
  unread: boolean;
  /** Ascending by ts. */
  messages: Message[];
}

export interface Seed {
  now: number;
  chats: MockChat[];
  rules: Rule[];
  automationLog: AutomationEvent[];
  logs: LogLine[];
  issues: Issue[];
  jobs: JobRecord[];
}

export const CONTACTS: Contact[] = [
  { name: "Ananya Rao", phone: "919845012345" },
  { name: "Mom ❤️", phone: "919880011223" },
  { name: "राहुल शर्मा", phone: "919811122334" },
  { name: "Karthik Iyer", phone: "919900155667" },
  { name: "Priya Patel", phone: "919820044556" },
  { name: "Dad", phone: "919880099887" },
  { name: "Arjun Mehta", phone: "919167788990" },
  { name: "प्रिया वर्मा", phone: "919711223344" },
  { name: "Sneha 🌸", phone: "919036655443" },
  { name: "Vikram Singh", phone: "919873344556" },
  { name: "Meera Nair", phone: "919447766554" },
  { name: "Rohan Gupta", phone: "919999012345" },
  { name: "Divya Krishnan", phone: "919884433221" },
  { name: "Aditya Joshi", phone: "919822233445" },
  { name: "Fatima Sheikh", phone: "919769988776" },
  { name: "Sam Wilson", phone: "14155550123" },
  { name: "Lena Müller", phone: "4915123456789" },
  { name: "अमित कुमार", phone: "919654321098" },
  { name: "Neha Kapoor", phone: "919810987654" },
  { name: "Siddharth Rao", phone: "919742211009" },
  { name: "Kavya Reddy", phone: "919959988771" },
  { name: "Landlord — Mr. Shetty", phone: "919845998877" },
  { name: "Dr. Iyengar Clinic", phone: "918041234567" },
  { name: "Ravi (Plumber)", phone: "919741122334" },
  { name: "Tanvi 🎨", phone: "919167711223" },
  { name: "James O'Connor", phone: "447700900123" },
  { name: "Ishaan", phone: "919008877665" },
  { name: "919731234567", phone: "919731234567" },
  { name: "Pooja Desai", phone: "919898765432" },
  { name: "Harsh Vardhan", phone: "919555123123" },
  { name: "Zoya Khan", phone: "919322334455" },
  { name: "Manoj Uncle", phone: "919448800112" },
];

const GROUPS = [
  "Family 🏠", "College Friends 🎓", "Bengaluru Runners 🏃", "Project Phoenix", "Flat 4B Society",
  "Weekend Trek ⛰️", "दोस्तों का अड्डा", "Book Club 📚", "Office — Platform Team", "Cricket Sunday 🏏",
  "Startup Founders India", "Cousins 👨‍👩‍👧‍👦",
];

/** LID-only participants (no phone mapping available). */
const LID_SENDERS = ["204958372615402@lid", "118273645509871@lid", "99887766554433@lid"];

const LINES = [
  "Hey! Are we still on for tonight?",
  "Yes, 8 pm works. See you there 👍",
  "Can you send me the invoice when you get a chance?",
  "On my way, stuck in traffic near Silk Board 😩",
  "Happy birthday!! 🎉🎂 Have an amazing year",
  "कल मिलते हैं, ठीक है?",
  "मैं घर पहुँच गया हूँ 🙏",
  "आज बहुत गर्मी है 🥵",
  "Did you see the match yesterday? Unbelievable finish 🏏",
  "Check this out: https://github.com/tauri-apps/tauri",
  "The menu is here www.example.com/menu — order whatever you like",
  "Meeting moved to 3:30, same link",
  "Lol 😂😂",
  "Thanks a lot, really appreciate it 🙌",
  "Can someone share the photos from Sunday?",
  "I'll call you in 10 mins",
  "Sure, no problem",
  "Ok",
  "👍",
  "Good morning everyone ☀️",
  "Reminder: rent is due on the 5th",
  "Please review the doc before tomorrow's standup",
  "Pushed the fix, CI is green ✅",
  "Who's bringing snacks for the trek?",
  "Train is delayed by 40 minutes 🚆",
  "नमस्ते! आप कैसे हैं? 😊",
  "Let's split it — ₹1,250 each",
  "Family dinner on Saturday, everyone please confirm",
  "The plumber will come at 11 tomorrow",
  "Can you pick up milk on the way back? 🥛",
  "Sending the boarding pass now ✈️",
  "Urgent: please call me when you see this",
  "Long message incoming… So I was thinking about the plan for next month. We could start the trek from the north base camp, stay two nights, and come back via the lake route. It's longer but the views are supposed to be incredible. Thoughts?",
  "Haha that's hilarious 🤣",
  "Where did you keep the car keys? 🔑",
  "Book club pick for next month: The Remains of the Day 📖",
  "5k done in 27:40 💪 new PB!",
  "Parcel delivered at the gate",
  "Doctor's appointment confirmed for Thursday 10:15",
  "Let me know if you need anything",
  "Sounds good!",
  "Can't make it today, sorry 😔",
  "👨‍👩‍👧 family photo time!",
  "Invoice attached for March, please process",
  "Weather looks clear for Sunday 🌤️",
  "Standup notes are in the doc",
  "Mom says call her back",
  "Here's the address: 42, 3rd Cross, Indiranagar",
  "Agreed 💯",
  "बधाई हो! 🎊",
];

const DOCS = [
  "Invoice_March_2025.pdf", "Lease Agreement.pdf", "Trip itinerary.docx", "Budget 2025.xlsx",
  "Pitch deck v3.pptx", "Resume - Priya.pdf", "Menu.pdf", "notes.txt", "photos-backup.zip",
  "Meeting minutes.docx", "Q3 report.pdf", "corrupt_scan.pdf", "Electricity bill Oct.pdf", "सूची.pdf",
];
const STICKERS = ["😺", "🐶", "🎉", "🥳", "🤖", "🌈", "🍕", "👻"];
const REACTIONS = ["👍", "❤️", "😂", "😮", "😢", "🙏", "🔥", "🎉"];

const MIME: Record<string, string> = {
  pdf: "application/pdf",
  docx: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
  xlsx: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
  pptx: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
  txt: "text/plain",
  zip: "application/zip",
};

export function phoneJid(phone: string): string {
  return `${phone}@s.whatsapp.net`;
}

function msgId(chatIdx: number, n: number): string {
  const h = hashString(`${chatIdx}:${n}`).toString(16).toUpperCase().padStart(8, "0");
  const h2 = hashString(`${n}:${chatIdx}:x`).toString(16).toUpperCase().padStart(8, "0");
  return `3EB0${h}${h2}`;
}

/** Build a MediaRef as the backend would once a file is local (or missing). */
export function buildMedia(kind: MediaKind, filename: string, size: number, hue: number, available: boolean): MediaRef {
  const ext = filename.split(".").pop()?.toLowerCase() ?? "";
  let url: string | null = null;
  let thumb: string | null = null;
  let mime: string;
  switch (kind) {
    case "image": {
      const dims: [number, number][] = [[800, 600], [600, 800], [960, 540], [700, 700]];
      const [w, h] = dims[hue % dims.length] ?? [800, 600];
      url = photoSvg(w, h, hue, filename);
      thumb = url;
      mime = "image/jpeg";
      break;
    }
    case "video":
      url = MOCK_VIDEO_URL;
      thumb = videoPosterSvg(hue, filename);
      mime = "video/webm";
      break;
    case "audio":
      url = MOCK_AUDIO_URL;
      mime = "audio/ogg";
      break;
    case "sticker":
      url = stickerSvg(STICKERS[hue % STICKERS.length] ?? "😺");
      thumb = url;
      mime = "image/webp";
      break;
    case "document":
      mime = MIME[ext] ?? "application/octet-stream";
      if (ext === "pdf") {
        url = MOCK_PDF_URL;
        thumb = pdfThumbSvg(filename);
      } else if (ext === "txt") {
        url = textDataUrl("Shopping list\n- milk\n- eggs\n- दाल\n- coffee ☕\n");
      } else {
        url = MOCK_PDF_URL; // stands in for the original file
      }
      break;
  }
  return {
    kind,
    filename,
    size,
    mime,
    source: available ? (hue % 7 === 0 ? "phone" : "store") : "missing",
    url: available ? url : null,
    thumb_url: available ? thumb : null,
  };
}

interface Gen {
  rnd: () => number;
  pick: <T>(a: readonly T[]) => T;
  int: (lo: number, hi: number) => number;
}

function gen(seed: number): Gen {
  const rnd = mulberry32(seed);
  return {
    rnd,
    pick: <T,>(a: readonly T[]) => a[Math.floor(rnd() * a.length)] as T,
    int: (lo, hi) => lo + Math.floor(rnd() * (hi - lo + 1)),
  };
}

function makeMedia(g: Gen, chatIdx: number, n: number, ts: number): { media: MediaRef; caption: string } {
  const r = g.rnd();
  const kind: MediaKind = r < 0.42 ? "image" : r < 0.52 ? "video" : r < 0.66 ? "audio" : r < 0.88 ? "document" : "sticker";
  const hue = (chatIdx * 37 + n * 53) % 360;
  const d = new Date(ts * 1000);
  const stamp = `${d.getFullYear()}${String(d.getMonth() + 1).padStart(2, "0")}${String(d.getDate()).padStart(2, "0")}`;
  let filename: string;
  let size: number;
  switch (kind) {
    case "image":
      filename = `IMG-${stamp}-WA${String(n % 10000).padStart(4, "0")}.jpg`;
      size = g.int(80_000, 3_500_000);
      break;
    case "video":
      filename = `VID-${stamp}-WA${String(n % 10000).padStart(4, "0")}.mp4`;
      size = g.int(2_000_000, 48_000_000);
      break;
    case "audio":
      filename = `PTT-${stamp}-WA${String(n % 10000).padStart(4, "0")}.opus`;
      size = g.int(8_000, 900_000);
      break;
    case "sticker":
      filename = `STK-${stamp}-WA${String(n % 10000).padStart(4, "0")}.webp`;
      size = g.int(10_000, 60_000);
      break;
    default:
      filename = g.pick(DOCS);
      size = g.int(20_000, 9_000_000);
  }
  const available = filename.startsWith("corrupt") ? false : g.rnd() > 0.24;
  const media = buildMedia(kind, filename, size, hue, available);
  const caption = kind === "image" && g.rnd() < 0.3 ? g.pick(["Look at this 😍", "From the trip", "सुंदर दृश्य", "Sunset today 🌅"]) : "";
  return { media, caption };
}

export function generateSeed(nowSec: number = Math.floor(Date.now() / 1000), seed = 42): Seed {
  const g = gen(seed);
  const chats: MockChat[] = [];
  const direct = CONTACTS.slice(0, 28);
  // Interleave groups among directs so the list mixes both.
  const order: ({ kind: "d"; c: Contact } | { kind: "g"; name: string })[] = [];
  let gi = 0;
  direct.forEach((c, i) => {
    order.push({ kind: "d", c });
    if (i % 2 === 1 && gi < GROUPS.length) order.push({ kind: "g", name: GROUPS[gi++] as string });
  });
  while (gi < GROUPS.length) order.push({ kind: "g", name: GROUPS[gi++] as string });

  order.forEach((entry, idx) => {
    const isGroup = entry.kind === "g";
    const jid = isGroup ? `1203630${String(40000000000 + idx * 7919)}@g.us` : phoneJid(entry.c.phone);
    const name = isGroup ? entry.name : entry.c.name;
    const count = idx === 0 ? 600 : idx === 2 ? 400 : g.int(18, 86);
    const members = isGroup
      ? [...Array.from({ length: g.int(4, 9) }, () => g.pick(CONTACTS)), ...(idx % 3 === 0 ? LID_SENDERS : [LID_SENDERS[0] as string])]
      : [];
    // Most recent activity first in idx order; the newest chats are minutes old.
    let ts = nowSec - Math.floor(idx * idx * 900 + g.int(30, 1800));
    const msgs: Message[] = [];
    for (let n = count - 1; n >= 0; n--) {
      const fromMe = g.rnd() < (isGroup ? 0.2 : 0.42);
      let senderJid: string | null;
      let senderName: string;
      if (fromMe) {
        senderJid = null;
        senderName = "Me";
      } else if (isGroup) {
        const m = g.pick(members);
        if (typeof m === "string") {
          senderJid = m;
          senderName = m.split("@")[0] ?? m;
        } else {
          senderJid = phoneJid(m.phone);
          senderName = m.name;
        }
      } else {
        const c = (entry as { c: Contact }).c;
        senderJid = phoneJid(c.phone);
        senderName = c.name;
      }
      const hasMedia = g.rnd() < 0.13;
      const mm = hasMedia ? makeMedia(g, idx, n, ts) : null;
      const content = mm ? mm.caption : g.pick(LINES);
      msgs.push({
        id: msgId(idx, n),
        chat_jid: jid,
        sender_jid: senderJid,
        sender_name: senderName,
        is_from_me: fromMe,
        ts,
        content,
        media: mm ? mm.media : null,
        quoted: null,
        reactions: [],
        deleted: false,
      });
      // Gap to the previous (older) message: bursts of short gaps, occasional long ones.
      const r = g.rnd();
      ts -= r < 0.6 ? g.int(20, 600) : r < 0.9 ? g.int(600, 14_400) : g.int(14_400, 3 * 86_400);
    }
    msgs.reverse();
    // Post-process: quotes, reactions, deletions.
    msgs.forEach((m, i) => {
      if (i > 3 && g.rnd() < 0.08) {
        const q = msgs[i - g.int(1, Math.min(15, i))];
        if (q && !q.deleted) {
          m.quoted = {
            id: q.id,
            sender_name: q.is_from_me ? "You" : q.sender_name,
            preview: q.content || (q.media ? `${mediaEmoji(q.media.kind)} ${q.media.filename ?? q.media.kind}` : ""),
          };
        }
      }
      if (g.rnd() < 0.1) {
        const k = g.int(1, 3);
        for (let j = 0; j < k; j++) {
          const fromMe = j === 0 && g.rnd() < 0.3;
          m.reactions.push({
            emoji: g.pick(REACTIONS),
            sender_name: fromMe ? "Me" : isGroup ? g.pick(CONTACTS).name : name,
            is_from_me: fromMe,
          });
        }
      }
      if (g.rnd() < 0.015) {
        m.deleted = true;
        m.content = "";
        m.media = null;
        m.reactions = [];
      }
    });
    const last = msgs[msgs.length - 1];
    chats.push({
      jid,
      name,
      is_group: isGroup,
      unread: (idx % 5 === 1 || idx === 3) && !!last,
      messages: msgs,
    });
  });

  return {
    now: nowSec,
    chats,
    rules: seedRules(nowSec, chats),
    automationLog: seedAutomationLog(nowSec, chats),
    logs: seedLogs(nowSec, g),
    issues: seedIssues(nowSec),
    jobs: seedJobs(nowSec),
  };
}

export { mediaEmoji };

function seedRules(now: number, chats: MockChat[]): Rule[] {
  const fam = chats.find((c) => c.name.startsWith("Family"));
  return [
    {
      id: 1,
      name: "Night auto-reply for urgent messages",
      enabled: true,
      dry_run: true,
      trigger: {
        chat_jids: [],
        include_groups: false,
        senders: [],
        keywords: ["urgent", "call me"],
        regex: null,
        media_kinds: [],
        active_from: "23:00",
        active_to: "07:00",
        mention_only: false,
      },
      actions: [
        { type: "reply", text: "Hi {sender_name}, I'm asleep — will call back in the morning. (auto-reply, {time})" },
        { type: "notify", title: "Urgent from {sender_name}" },
      ],
      cooldown_secs: 1800,
      fired_count: 7,
      last_fired_at: now - 3 * 86_400,
    },
    {
      id: 2,
      name: "Save invoices",
      enabled: true,
      dry_run: false,
      trigger: {
        chat_jids: [],
        include_groups: false,
        senders: [],
        keywords: ["invoice"],
        regex: "(?:INV|inv)[-_ ]?\\d+",
        media_kinds: ["document"],
        active_from: null,
        active_to: null,
        mention_only: false,
      },
      actions: [{ type: "download_media" }, { type: "tag", label: "finance" }],
      cooldown_secs: 0,
      fired_count: 23,
      last_fired_at: now - 7200,
    },
    {
      id: 3,
      name: "Family group mentions",
      enabled: false,
      dry_run: true,
      trigger: {
        chat_jids: fam ? [fam.jid] : [],
        include_groups: true,
        senders: [],
        keywords: [],
        regex: null,
        media_kinds: [],
        active_from: null,
        active_to: null,
        mention_only: true,
      },
      actions: [{ type: "notify", title: null }, { type: "webhook", url: "https://hooks.example.com/wa" }],
      cooldown_secs: 600,
      fired_count: 0,
      last_fired_at: null,
    },
  ];
}

function seedAutomationLog(now: number, chats: MockChat[]): AutomationEvent[] {
  const out: AutomationEvent[] = [];
  for (let i = 0; i < 8; i++) {
    const chat = chats[(i * 5) % chats.length] as MockChat;
    const m = chat.messages[chat.messages.length - 1 - i] ?? chat.messages[0];
    const night = i % 2 === 0;
    out.push({
      rule_id: night ? 1 : 2,
      rule_name: night ? "Night auto-reply for urgent messages" : "Save invoices",
      chat_jid: chat.jid,
      message_id: m?.id ?? "",
      dry_run: night,
      outcomes: night ? ["[dry-run] would reply", "[dry-run] would notify"] : ["downloaded Invoice_March_2025.pdf", "tagged finance"],
      at: now - i * 5400 - 600,
    });
  }
  return out;
}

const LOG_TARGETS = ["wa_bridge", "wa_db", "wa_db::search", "wa_jobs::backfill", "wa_automation", "wa_obs", "wa_desk::ipc", "wa_jobs::convert"];

export function randomLogLine(g: () => number, ts: number): LogLine {
  const r = g();
  const level: LogLevel = r < 0.05 ? "error" : r < 0.14 ? "warn" : r < 0.62 ? "info" : r < 0.92 ? "debug" : "trace";
  const target = LOG_TARGETS[Math.floor(g() * LOG_TARGETS.length)] ?? "wa_desk";
  const templates: Record<LogLevel, string[]> = {
    error: ["bridge request failed", "thumbnail generation failed", "ffmpeg exited with status 1", "media download failed"],
    warn: ["bridge slow to respond", "sqlite busy, retrying", "phone media not found for message", "search index lagging behind"],
    info: ["polled new messages", "job progress", "connected to bridge", "rule evaluated", "settings saved", "chat list refreshed"],
    debug: ["query took", "cache hit", "range request served", "watermark advanced"],
    trace: ["tick", "ipc call", "event emitted"],
  };
  const message = (templates[level][Math.floor(g() * templates[level].length)] ?? "event");
  const fields =
    level === "debug" ? `{"ms":${Math.floor(g() * 280)}}`
    : level === "error" ? `{"err":"${target === "wa_bridge" ? "connection refused (os error 111)" : "exit status: 1"}"}`
    : level === "info" && message === "polled new messages" ? `{"count":${Math.floor(g() * 6)},"chats":${1 + Math.floor(g() * 3)}}`
    : "";
  return { ts, level, target, message, fields };
}

function seedLogs(now: number, g: Gen): LogLine[] {
  const out: LogLine[] = [];
  let ts = now - 7200;
  for (let i = 0; i < 420; i++) {
    ts += g.int(2, 30);
    out.push(randomLogLine(g.rnd, Math.min(ts, now)));
  }
  return out.reverse(); // newest first
}

function seedIssues(now: number): Issue[] {
  return [
    {
      id: 1, fingerprint: "a1f3c9", title: "Bridge connection refused", kind: "bridge_down", source: "bridge",
      count: 14, first_seen: now - 5 * 86_400, last_seen: now - 1800,
      last_detail: "GET http://127.0.0.1:8080/api/status: connection refused (os error 111)", status: "open",
    },
    {
      id: 2, fingerprint: "b7d210", title: "Phone sync failed: no USB device", kind: "external", source: "jobs.phone_sync",
      count: 3, first_seen: now - 2 * 86_400, last_seen: now - 86_400,
      last_detail: "adb: no devices/emulators found", status: "open",
    },
    {
      id: 3, fingerprint: "c044e8", title: "Thumbnail generation failed for 3 videos", kind: "external", source: "jobs.thumbnails",
      count: 3, first_seen: now - 86_400 * 3, last_seen: now - 86_400 * 3 + 120,
      last_detail: "ffmpeg: moov atom not found", status: "open",
    },
    {
      id: 4, fingerprint: "d9e5aa", title: "Search index rebuild timed out", kind: "timeout", source: "db.search",
      count: 1, first_seen: now - 9 * 86_400, last_seen: now - 9 * 86_400, last_detail: null, status: "resolved",
    },
  ];
}

function seedJobs(now: number): JobRecord[] {
  return [
    {
      id: 4, spec: { type: "reindex" }, label: "Rebuild search index", status: "cancelled", done: 1200, total: 3000,
      phase: "indexing", message: "Cancelled by user", error: null, outputs: [], created_at: now - 3600 * 5, finished_at: now - 3600 * 5 + 40,
    },
    {
      id: 3,
      spec: { type: "convert", input: { type: "file", path: "/home/user/Documents/Lease.docx" }, kind: { type: "office_to_pdf" }, output_dir: null },
      label: "Convert Lease.docx → PDF", status: "succeeded", done: 1, total: 1, phase: "done", message: null, error: null,
      outputs: [`${MOCK_PDF_URL}?path=${encodeURIComponent("/home/user/.local/share/wa-desk/exports/Lease.pdf")}`],
      created_at: now - 86_400, finished_at: now - 86_400 + 6,
    },
    {
      id: 2, spec: { type: "phone_sync" }, label: "Phone sync (USB)", status: "failed", done: 0, total: 0, phase: "connecting",
      message: null, error: "adb: no devices/emulators found", outputs: [], created_at: now - 86_400 * 1.2, finished_at: now - 86_400 * 1.2 + 3,
    },
    {
      id: 1, spec: { type: "backfill", days: 30, kinds: ["image", "document"], workers: 4, retry_failed: false },
      label: "Backfill media (30 days)", status: "succeeded", done: 212, total: 212, phase: "done", message: "212 files downloaded",
      error: null, outputs: [], created_at: now - 86_400 * 2, finished_at: now - 86_400 * 2 + 640,
    },
  ];
}
