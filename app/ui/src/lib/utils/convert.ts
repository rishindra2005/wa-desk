// Which conversions make sense for a given media item, with sensible defaults.

import type { ConvertKind } from "../bindings/ConvertKind";
import type { MediaKind } from "../bindings/MediaKind";
import { docBucket, extOf } from "./text";

export interface ConvertOption {
  id: string;
  label: string;
  hint: string;
  kind: ConvertKind;
}

/** Options valid for a media item (empty when nothing can be converted). */
export function convertOptions(kind: MediaKind, filename: string | null): ConvertOption[] {
  const ext = extOf(filename);
  switch (kind) {
    case "document":
      if (ext === "pdf") {
        return [
          { id: "pdf_compress", label: "Compress PDF", hint: "Smaller file (ebook quality)", kind: { type: "pdf_compress", level: "ebook" } },
          { id: "pdf_to_images", label: "PDF → images", hint: "One PNG per page, 150 dpi", kind: { type: "pdf_to_images", dpi: 150 } },
        ];
      }
      if (docBucket(filename) === "office") {
        return [{ id: "office_to_pdf", label: "Convert to PDF", hint: "Word, Excel, PowerPoint, ODF", kind: { type: "office_to_pdf" } }];
      }
      return [];
    case "image":
      return [
        { id: "image_jpeg", label: "Compress (JPEG)", hint: "Quality 80, max 2048 px", kind: { type: "image_compress", quality: 80, max_dim: 2048, format: "jpeg" } },
        { id: "image_webp", label: "Convert to WebP", hint: "Quality 80, max 2048 px", kind: { type: "image_compress", quality: 80, max_dim: 2048, format: "webp" } },
        { id: "image_png", label: "Convert to PNG", hint: "Lossless, max 2048 px", kind: { type: "image_compress", quality: 100, max_dim: 2048, format: "png" } },
      ];
    case "video":
      return [
        { id: "video_720", label: "Compress video (720p)", hint: "H.264, CRF 28", kind: { type: "video_compress", crf: 28, max_height: 720 } },
        { id: "video_480", label: "Compress video (480p)", hint: "H.264, CRF 30", kind: { type: "video_compress", crf: 30, max_height: 480 } },
      ];
    case "audio":
      return [{ id: "audio_opus", label: "Convert to Opus", hint: "32 kbps, voice-friendly", kind: { type: "audio_to_opus", bitrate_kbps: 32 } }];
    case "sticker":
      return [];
  }
}

/** Editable parameters for "More options", by kind. */
export interface ConvertParam {
  key: string;
  label: string;
  min: number;
  max: number;
  step: number;
}

export function convertParams(kind: ConvertKind): ConvertParam[] {
  switch (kind.type) {
    case "image_compress": return [
      { key: "quality", label: "Quality", min: 20, max: 100, step: 5 },
      { key: "max_dim", label: "Max size (px)", min: 256, max: 8192, step: 128 },
    ];
    case "video_compress": return [
      { key: "crf", label: "CRF (lower = better)", min: 18, max: 40, step: 1 },
      { key: "max_height", label: "Max height (px)", min: 240, max: 2160, step: 120 },
    ];
    case "audio_to_opus": return [{ key: "bitrate_kbps", label: "Bitrate (kbps)", min: 12, max: 128, step: 4 }];
    case "pdf_to_images": return [{ key: "dpi", label: "Resolution (dpi)", min: 72, max: 300, step: 6 }];
    default: return [];
  }
}
