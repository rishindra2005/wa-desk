// Placeholder media for the mock backend: SVG data URLs and small files in public/mock/.

export const MOCK_PDF_URL = "/mock/sample.pdf";
export const MOCK_VIDEO_URL = "/mock/sample.webm";
export const MOCK_AUDIO_URL = "/mock/sample.ogg";

function svgUrl(svg: string): string {
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}

function esc(s: string): string {
  return s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c] ?? c);
}

/** A landscape-ish "photo": sky gradient, sun and hills, with a label. */
export function photoSvg(w: number, h: number, hue: number, label: string): string {
  const h2 = (hue + 40) % 360;
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}">
<defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="hsl(${hue},70%,72%)"/><stop offset="1" stop-color="hsl(${h2},60%,88%)"/></linearGradient></defs>
<rect width="${w}" height="${h}" fill="url(#g)"/>
<circle cx="${w * 0.72}" cy="${h * 0.3}" r="${Math.min(w, h) * 0.11}" fill="hsl(${(hue + 180) % 360},90%,85%)"/>
<path d="M0 ${h * 0.75} Q ${w * 0.25} ${h * 0.5} ${w * 0.5} ${h * 0.72} T ${w} ${h * 0.62} V ${h} H 0 Z" fill="hsl(${(hue + 100) % 360},35%,42%)"/>
<path d="M0 ${h * 0.85} Q ${w * 0.4} ${h * 0.7} ${w} ${h * 0.86} V ${h} H 0 Z" fill="hsl(${(hue + 120) % 360},40%,30%)"/>
<text x="16" y="${h - 16}" font-family="sans-serif" font-size="${Math.round(Math.min(w, h) / 18)}" fill="rgba(255,255,255,.85)">${esc(label)}</text>
</svg>`;
  return svgUrl(svg);
}

/** A video poster with a play glyph. */
export function videoPosterSvg(hue: number, label: string): string {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="320" height="180" viewBox="0 0 320 180">
<rect width="320" height="180" fill="hsl(${hue},35%,22%)"/>
<rect x="0" y="120" width="320" height="60" fill="hsl(${hue},40%,30%)"/>
<circle cx="160" cy="90" r="28" fill="rgba(255,255,255,.85)"/><path d="M152 76 L174 90 L152 104 Z" fill="hsl(${hue},35%,22%)"/>
<text x="12" y="168" font-family="sans-serif" font-size="12" fill="rgba(255,255,255,.8)">${esc(label)}</text></svg>`;
  return svgUrl(svg);
}

/** A sticker: big emoji on a transparent background. */
export function stickerSvg(emoji: string): string {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="160" height="160" viewBox="0 0 160 160"><text x="80" y="112" text-anchor="middle" font-size="104">${emoji}</text></svg>`;
  return svgUrl(svg);
}

/** First-page-like thumbnail for a PDF. */
export function pdfThumbSvg(title: string): string {
  const lines = Array.from({ length: 9 }, (_, i) => `<rect x="24" y="${70 + i * 16}" width="${150 - (i % 3) * 30}" height="6" rx="3" fill="#cbd5e1"/>`).join("");
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="200" height="260" viewBox="0 0 200 260"><rect width="200" height="260" fill="#fff"/><rect x="24" y="28" width="120" height="14" rx="3" fill="#0f172a"/><text x="24" y="58" font-family="sans-serif" font-size="9" fill="#475569">${esc(title.slice(0, 30))}</text>${lines}<rect x="24" y="220" width="152" height="18" rx="3" fill="#10b981"/></svg>`;
  return svgUrl(svg);
}

/** Pairing QR placeholder: a deterministic pseudo-QR pattern as an SVG data URL. */
export function qrSvg(seed: number): string {
  const n = 29;
  let rects = "";
  let s = seed >>> 0;
  const rnd = () => {
    s = (Math.imul(s, 1664525) + 1013904223) >>> 0;
    return s / 4294967296;
  };
  const finder = (x: number, y: number) =>
    `<rect x="${x}" y="${y}" width="7" height="7" fill="#000"/><rect x="${x + 1}" y="${y + 1}" width="5" height="5" fill="#fff"/><rect x="${x + 2}" y="${y + 2}" width="3" height="3" fill="#000"/>`;
  for (let y = 0; y < n; y++) {
    for (let x = 0; x < n; x++) {
      const inFinder = (x < 8 && y < 8) || (x > n - 9 && y < 8) || (x < 8 && y > n - 9);
      if (!inFinder && rnd() > 0.52) rects += `<rect x="${x}" y="${y}" width="1" height="1"/>`;
    }
  }
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="-2 -2 ${n + 4} ${n + 4}" shape-rendering="crispEdges"><rect x="-2" y="-2" width="${n + 4}" height="${n + 4}" fill="#fff"/>${rects}${finder(0, 0)}${finder(n - 7, 0)}${finder(0, n - 7)}</svg>`;
  return svgUrl(svg);
}

export function textDataUrl(text: string): string {
  return `data:text/plain;charset=utf-8,${encodeURIComponent(text)}`;
}
