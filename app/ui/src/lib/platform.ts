// Thin wrappers over Tauri plugins (dialog, opener, notification) with
// normalised errors. All go through IPC, so the mock backend answers in dev.

import { open, save } from "@tauri-apps/plugin-dialog";
import { openUrl, openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
import { toAppError } from "./api";

export interface FileFilter {
  name: string;
  extensions: string[];
}

export const FILTERS = {
  any: [] as FileFilter[],
  office: [{ name: "Office documents", extensions: ["doc", "docx", "odt", "rtf", "xls", "xlsx", "ods", "ppt", "pptx", "odp"] }],
  image: [{ name: "Images", extensions: ["jpg", "jpeg", "png", "webp", "heic", "gif", "bmp", "tiff"] }],
  video: [{ name: "Videos", extensions: ["mp4", "mov", "mkv", "webm", "avi", "3gp"] }],
  audio: [{ name: "Audio", extensions: ["mp3", "m4a", "ogg", "opus", "wav", "aac", "flac"] }],
  pdf: [{ name: "PDF", extensions: ["pdf"] }],
};

/** Pick one local file; resolves null when cancelled. */
export async function pickFile(title: string, filters: FileFilter[] = []): Promise<string | null> {
  try {
    const r = await open({ title, multiple: false, directory: false, ...(filters.length ? { filters } : {}) });
    return typeof r === "string" ? r : null;
  } catch (e) {
    throw toAppError(e);
  }
}

/** Pick a folder; resolves null when cancelled. */
export async function pickFolder(title: string): Promise<string | null> {
  try {
    const r = await open({ title, directory: true, multiple: false });
    return typeof r === "string" ? r : null;
  } catch (e) {
    throw toAppError(e);
  }
}

/** Ask where to save; resolves null when cancelled. */
export async function pickSavePath(defaultName: string): Promise<string | null> {
  try {
    const r = await save({ title: "Save as", defaultPath: defaultName });
    return r ?? null;
  } catch (e) {
    throw toAppError(e);
  }
}

/** Open an http(s)/mailto link in the system browser. */
export async function openLink(url: string): Promise<void> {
  try {
    await openUrl(url);
  } catch (e) {
    throw toAppError(e);
  }
}

/** Open a local file (exports folder) with the default app. */
export async function openLocalPath(path: string): Promise<void> {
  try {
    await openPath(path);
  } catch (e) {
    throw toAppError(e);
  }
}

/** Reveal a local file in the file manager. */
export async function revealPath(path: string): Promise<void> {
  try {
    await revealItemInDir(path);
  } catch (e) {
    throw toAppError(e);
  }
}

/**
 * Absolute path of an exported/converted output from its URL:
 * `wa-media://localhost/x/<urlenc path>`, `http://wa-media.localhost/x/<…>`,
 * or (mock) any URL with a `?path=` query.
 */
export function outputPath(url: string): string | null {
  const m = /^(?:wa-media:\/\/localhost|https?:\/\/wa-media\.localhost)\/x\/(.+)$/.exec(url);
  if (m?.[1]) {
    try {
      return decodeURIComponent(m[1]);
    } catch {
      return null;
    }
  }
  try {
    const u = new URL(url, "http://local.invalid/");
    return u.searchParams.get("path");
  } catch {
    return null;
  }
}

/** Show a desktop notification if permitted (best effort, never throws). */
export async function notify(title: string, body?: string): Promise<void> {
  try {
    const mod = await import("@tauri-apps/plugin-notification");
    let ok = await mod.isPermissionGranted();
    if (!ok) ok = (await mod.requestPermission()) === "granted";
    if (ok) mod.sendNotification(body ? { title, body } : { title });
  } catch {
    /* notifications unavailable */
  }
}
