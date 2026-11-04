/** One row of the archive's tree. A parent always precedes its children. */
export interface TreeNode {
  parent: number | null;
  name: string;
  path: string;
  size: number;
  compressed_size: number;
  is_dir: boolean;
  file_count: number;
  total_size: number;
}

export interface RejectedEntry {
  name: string;
  reason: string;
}

export interface ScanResult {
  nodes: TreeNode[];
  rejected: RejectedEntry[];
  rejected_count: number;
  total_size: number;
  fetched_size: number;
}

export type DownloadTarget =
  | { kind: "file"; zipPath: string; dest: string }
  | { kind: "selection"; destDir: string; base: string; files: string[] };

export interface ActiveFile {
  path: string;
  done: number;
}

export type JobEvent =
  | { kind: "progress"; doneBytes: number; filesDone: number; active: ActiveFile[] }
  | { kind: "fileFailed"; path: string; error: string }
  | {
      kind: "finished";
      completed: number;
      failed: number;
      cancelled: boolean;
      writtenBytes: number;
      networkBytes: number;
    };
