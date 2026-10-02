import type { ViewId } from "../face";

/** Tapped pages and charts rest on their configured view after ten minutes. */
export function recentTap(tappedAt: string | null, now: Date, ttlMs = 10 * 60_000): boolean {
  if (tappedAt === null) return false;
  const at = Date.parse(tappedAt);
  return Number.isFinite(at) && now.getTime() - at <= ttlMs;
}

export function pageCount(length: number, perPage: number): number {
  return Math.max(1, Math.ceil(length / perPage));
}

export function pageViews(pages: number): ViewId[] {
  return Array.from({ length: pages }, (_, page) => (page === 0 ? "" : `page-${page + 1}`));
}

export function pageForView(view: ViewId | undefined, pages: number): number {
  const page = pageViews(pages).indexOf(view ?? "");
  return page === -1 ? 0 : page;
}
