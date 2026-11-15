/** Imperative handle exposed by VirtualList via `bind:api`. */
export interface VirtualListApi {
  scrollToKey(key: string, align?: "start" | "center" | "end"): boolean;
  scrollToBottom(): void;
  scrollToTop(): void;
  isAtBottom(): boolean;
}
