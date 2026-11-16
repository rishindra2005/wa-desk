// One pdf.js module + one worker for the whole app, loaded lazily and warmable ahead of time.

import type * as PdfJs from "pdfjs-dist/legacy/build/pdf.mjs";

interface Loaded {
  pdfjs: typeof PdfJs;
  worker: PdfJs.PDFWorker;
}

let cached: Promise<Loaded> | null = null;

/** Load pdf.js and create the shared worker (idempotent). */
export function getPdfjs(): Promise<Loaded> {
  cached ??= (async () => {
    const pdfjs = await import("pdfjs-dist/legacy/build/pdf.mjs");
    if (!pdfjs.GlobalWorkerOptions.workerSrc) {
      const w = await import("pdfjs-dist/legacy/build/pdf.worker.min.mjs?url");
      pdfjs.GlobalWorkerOptions.workerSrc = w.default;
    }
    return { pdfjs, worker: new pdfjs.PDFWorker() };
  })();
  cached.catch(() => (cached = null));
  return cached;
}

/** Start loading pdf.js when the browser is idle so the first PDF preview opens instantly. */
export function warmPdfjs(): void {
  // In dev, pdf.js's bundled polyfills trip Svelte's array-proxy warning; production only.
  if (!import.meta.env.PROD) return;
  const run = () => void getPdfjs().catch(() => {});
  const w = window as unknown as { requestIdleCallback?: (cb: () => void) => void };
  if (w.requestIdleCallback) w.requestIdleCallback(run);
  else setTimeout(run, 1500);
}
