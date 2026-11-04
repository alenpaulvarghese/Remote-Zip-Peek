export function formatBytes(bytes: number, decimals = 1): string {
  if (!+bytes) return "0 B";
  const k = 1024;
  const dm = decimals < 0 ? 0 : decimals;
  const sizes = ["B", "KB", "MB", "GB", "TB", "PB"];
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(k)), sizes.length - 1);
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(dm))} ${sizes[i]}`;
}

export const MIN_WORKERS = 1;
export const MAX_WORKERS = 20;
const DEFAULT_WORKERS = 10;

/** An emptied number field is null, and typed values can be out of range. */
export function normalizeWorkers(value: number | null | undefined): number {
  if (value === null || value === undefined || !Number.isFinite(value)) return DEFAULT_WORKERS;
  return Math.min(MAX_WORKERS, Math.max(MIN_WORKERS, Math.round(value)));
}

export function formatCount(count: number): string {
  return count.toLocaleString("en-US");
}

export function plural(count: number, noun: string, pluralNoun = `${noun}s`): string {
  return `${formatCount(count)} ${count === 1 ? noun : pluralNoun}`;
}

/** Share of the archive not fetched. Fetching more than the archive's size counts as nothing saved. */
export function calcEfficiency(total: number, fetched: number): string {
  if (total === 0) return "0";
  return Math.max(0, ((total - fetched) / total) * 100).toFixed(1);
}
