<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { save, open, ask } from "@tauri-apps/plugin-dialog";

  interface FileNode {
    name: string;
    path: string;
    size: number;
    compressed_size: number;
    is_dir: boolean;
    children: FileNode[];
  }

  interface ScanResult {
    files: FileNode[];
    total_size: number;
    fetched_size: number;
  }

  interface DownloadProgress {
    current_file: string;
    completed: number;
    total: number;
  }

  let url = $state("");
  let scanResult: ScanResult | null = $state(null);
  let isLoading = $state(false);
  let errorMsg = $state("");
  let downloadingPaths = $state<Set<string>>(new Set());
  let expandedDirs = $state<Set<string>>(new Set());

  // Folder download progress
  let folderProgress: DownloadProgress | null = $state(null);
  let folderDownloadPath = $state("");

  // Settings
  let downloadWorkers = $state(10);
  let showSettings = $state(false);

  // Success toast
  let toastMsg = $state("");
  let toastTimer: ReturnType<typeof setTimeout> | null = $state(null);

  function showToast(msg: string) {
    if (toastTimer) clearTimeout(toastTimer);
    toastMsg = msg;
    toastTimer = setTimeout(() => { toastMsg = ""; toastTimer = null; }, 4000);
  }

  async function scanZip(event: Event) {
    event.preventDefault();
    if (!url) return;

    isLoading = true;
    errorMsg = "";
    scanResult = null;
    expandedDirs = new Set();

    try {
      scanResult = await invoke("scan_zip", { url });
    } catch (err: any) {
      errorMsg = err.toString();
    } finally {
      isLoading = false;
    }
  }

  function toggleDir(path: string) {
    const next = new Set(expandedDirs);
    if (next.has(path)) {
      next.delete(path);
    } else {
      next.add(path);
    }
    expandedDirs = next;
  }

  function formatBytes(bytes: number, decimals = 1) {
    if (!+bytes) return "0 B";
    const k = 1024;
    const dm = decimals < 0 ? 0 : decimals;
    const sizes = ["B", "KB", "MB", "GB", "TB"];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return `${parseFloat((bytes / Math.pow(k, i)).toFixed(dm))} ${sizes[i]}`;
  }

  function calcEfficiency(total: number, fetched: number) {
    if (total === 0) return "0";
    return (((total - fetched) / total) * 100).toFixed(1);
  }

  function collectFiles(node: FileNode): string[] {
    if (!node.is_dir) return [node.path];
    const result: string[] = [];
    for (const child of node.children) {
      result.push(...collectFiles(child));
    }
    return result;
  }

  function countFiles(node: FileNode): number {
    if (!node.is_dir) return 1;
    let count = 0;
    for (const child of node.children) {
      count += countFiles(child);
    }
    return count;
  }

  function calcFolderSize(node: FileNode): number {
    if (!node.is_dir) return node.size;
    let total = 0;
    for (const child of node.children) {
      total += calcFolderSize(child);
    }
    return total;
  }

  async function downloadSingleFile(node: FileNode) {
    const savePath = await save({
      defaultPath: node.name,
      title: "Save file as...",
    });
    if (!savePath) return;

    const next = new Set(downloadingPaths);
    next.add(node.path);
    downloadingPaths = next;

    try {
      await invoke("download_file", {
        url,
        filePath: node.path,
        savePath,
      });
      showToast(`Downloaded "${node.name}" · ${formatBytes(node.size)}`);
    } catch (err: any) {
      errorMsg = `Download failed: ${err}`;
    } finally {
      const done = new Set(downloadingPaths);
      done.delete(node.path);
      downloadingPaths = done;
    }
  }

  function calcNodeSize(node: FileNode): number {
    if (!node.is_dir) return node.size;
    let s = 0;
    for (const c of node.children) s += calcNodeSize(c);
    return s;
  }

  async function downloadFolder(node: FileNode) {
    const saveDir = await open({
      directory: true,
      title: "Select destination folder",
    });
    if (!saveDir) return;

    // Create a subfolder with the folder's name
    const destDir = saveDir + "/" + node.name;

    let filePaths = collectFiles(node);
    if (filePaths.length === 0) return;

    // Build local destination paths and check which already exist
    const localPaths = filePaths.map((fp) => {
      const relative = fp.startsWith(node.path)
        ? fp.slice(node.path.length).replace(/^\//, "")
        : fp;
      return destDir + "/" + relative;
    });

    const existing = await invoke<string[]>("check_existing_files", { paths: localPaths });

    if (existing.length > 0) {
      const redownload = await ask(
        `${existing.length} of ${filePaths.length} files already exist in "${node.name}". Do you want to re-download them?`,
        { title: "Files Already Exist", kind: "warning", okLabel: "Re-download All", cancelLabel: "Skip Existing" }
      );

      if (!redownload) {
        // Build a set of zip paths that already exist locally
        const existingSet = new Set(existing);
        filePaths = filePaths.filter((_, i) => !existingSet.has(localPaths[i]));
        if (filePaths.length === 0) return; // everything already downloaded
      }
    }

    const next = new Set(downloadingPaths);
    next.add(node.path);
    downloadingPaths = next;
    folderDownloadPath = node.path;
    folderProgress = { current_file: "", completed: 0, total: filePaths.length };

    try {
      const unlisten = await listen<DownloadProgress>("download-progress", (event) => {
        folderProgress = event.payload;
      });

      try {
        await invoke("download_folder", {
          url,
          folderPath: node.path,
          saveDir: destDir,
          filePaths,
          workers: downloadWorkers,
        });
        showToast(`Downloaded ${filePaths.length} file${filePaths.length !== 1 ? "s" : ""} · ${formatBytes(calcNodeSize(node))}`);
      } finally {
        unlisten();
      }
    } catch (err: any) {
      errorMsg = `Download failed: ${err}`;
    } finally {
      const done = new Set(downloadingPaths);
      done.delete(node.path);
      downloadingPaths = done;
      folderProgress = null;
      folderDownloadPath = "";
    }
  }
</script>

<div class="h-screen flex flex-col bg-bg-base font-sans select-none">
  <!-- Minimal top bar -->
  <header
    class="flex items-center h-11 px-4 bg-bg-surface border-b border-border-default shrink-0"
    style="-webkit-app-region: drag;"
  >
    <div class="flex items-center gap-2">
      <svg class="w-4 h-4 text-accent" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4" />
        <polyline points="7 10 12 15 17 10" />
        <line x1="12" y1="15" x2="12" y2="3" />
      </svg>
      <span class="text-sm font-semibold text-text-primary tracking-tight">Remote ZIP Explorer</span>
    </div>

    <!-- Settings toggle -->
    <div class="ml-auto flex items-center gap-2 relative" style="-webkit-app-region: no-drag;">
      <button
        class="p-1.5 rounded-md hover:bg-bg-hover/60 transition-colors text-text-tertiary hover:text-text-secondary"
        aria-label="Settings"
        onclick={() => (showSettings = !showSettings)}
      >
        <svg class="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <circle cx="12" cy="12" r="3" />
          <path d="M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 010 2.83 2 2 0 01-2.83 0l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 01-4 0v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 01-2.83-2.83l.06-.06A1.65 1.65 0 004.68 15a1.65 1.65 0 00-1.51-1H3a2 2 0 010-4h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 012.83-2.83l.06.06A1.65 1.65 0 009 4.68a1.65 1.65 0 001-1.51V3a2 2 0 014 0v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 012.83 2.83l-.06.06A1.65 1.65 0 0019.4 9a1.65 1.65 0 001.51 1H21a2 2 0 010 4h-.09a1.65 1.65 0 00-1.51 1z" />
        </svg>
      </button>

      {#if showSettings}
        <div class="absolute right-0 top-full mt-1 bg-bg-elevated border border-border-default rounded-lg shadow-lg p-3 z-50 w-52">
          <label class="flex items-center justify-between text-xs text-text-secondary">
            <span>Download Workers</span>
            <input
              type="number"
              min="1"
              max="20"
              bind:value={downloadWorkers}
              class="w-14 bg-bg-input text-text-primary text-xs px-2 py-1 rounded border border-border-default text-center outline-none focus:border-accent"
            />
          </label>
        </div>
      {/if}
    </div>
  </header>

  <!-- Main content -->
  <main class="flex-1 flex flex-col overflow-hidden">
    <!-- URL input area -->
    <div class="px-4 py-3 bg-bg-surface border-b border-border-default shrink-0">
      <form class="flex gap-2 items-center" onsubmit={scanZip}>
        <div class="flex-1 relative">
          <input
            type="url"
            bind:value={url}
            placeholder="Paste a remote .zip URL..."
            class="w-full bg-bg-input text-text-primary text-sm px-3 py-2 rounded-lg border border-border-default focus:border-accent focus:ring-1 focus:ring-accent/30 outline-none transition-all placeholder:text-text-tertiary"
            required
          />
        </div>
        <button
          type="submit"
          disabled={isLoading}
          class="bg-accent hover:bg-accent-hover text-bg-base text-sm font-semibold px-5 py-2 rounded-lg transition-all disabled:opacity-40 disabled:cursor-not-allowed flex items-center gap-2 shrink-0"
        >
          {#if isLoading}
            <svg class="animate-spin h-3.5 w-3.5" viewBox="0 0 24 24" fill="none">
              <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="3" />
              <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z" />
            </svg>
            Scanning…
          {:else}
            Scan
          {/if}
        </button>
      </form>

      {#if errorMsg}
        <div class="mt-2 px-3 py-2 bg-error-bg text-error text-xs rounded-lg border border-error/20 flex items-center justify-between">
          <span>{errorMsg}</span>
          <button class="ml-2 text-error/60 hover:text-error" onclick={() => (errorMsg = "")}>✕</button>
        </div>
      {/if}
    </div>

    {#if scanResult}
      <!-- Stats row -->
      <div class="flex gap-3 px-4 py-3 bg-bg-surface border-b border-border-default shrink-0">
        <div class="flex-1 bg-bg-card rounded-lg px-3 py-2 border border-border-default">
          <div class="text-[10px] uppercase tracking-widest text-text-tertiary font-medium">Total Size</div>
          <div class="text-sm font-semibold text-text-primary mt-0.5">{formatBytes(scanResult.total_size)}</div>
        </div>
        <div class="flex-1 bg-bg-card rounded-lg px-3 py-2 border border-border-default">
          <div class="text-[10px] uppercase tracking-widest text-text-tertiary font-medium">Fetched</div>
          <div class="text-sm font-semibold text-text-primary mt-0.5">{formatBytes(scanResult.fetched_size)}</div>
        </div>
        <div class="flex-1 bg-bg-card rounded-lg px-3 py-2 border border-border-default">
          <div class="text-[10px] uppercase tracking-widest text-text-tertiary font-medium">Efficiency</div>
          <div class="text-sm font-semibold text-success mt-0.5">{calcEfficiency(scanResult.total_size, scanResult.fetched_size)}% saved</div>
        </div>
      </div>

      <!-- Download progress bar -->
      {#if folderProgress}
        <div class="px-4 py-2 bg-bg-surface border-b border-border-default shrink-0">
          <div class="flex items-center gap-3 text-xs">
            <svg class="animate-spin h-3.5 w-3.5 text-accent shrink-0" viewBox="0 0 24 24" fill="none">
              <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="3" />
              <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z" />
            </svg>
            <div class="flex-1 min-w-0">
              <div class="flex justify-between text-text-secondary mb-1">
                <span class="truncate">{folderProgress.current_file.split('/').pop() || 'Starting...'}</span>
                <span class="shrink-0 ml-2 text-text-accent font-medium">{folderProgress.completed}/{folderProgress.total}</span>
              </div>
              <div class="w-full h-1 bg-bg-elevated rounded-full overflow-hidden">
                <div
                  class="h-full bg-accent rounded-full transition-all duration-300 ease-out"
                  style="width: {(folderProgress.completed / folderProgress.total) * 100}%"
                ></div>
              </div>
            </div>
          </div>
        </div>
      {/if}

      <!-- File tree -->
      <div class="flex-1 overflow-auto px-4 py-2">
        {#snippet renderNode(node: FileNode, depth: number)}
          {@const isExpanded = expandedDirs.has(node.path)}
          {@const isDownloading = downloadingPaths.has(node.path)}
          <div>
            <div
              class="group flex items-center gap-1.5 py-1 px-2 rounded-md hover:bg-bg-hover/60 transition-colors cursor-default text-sm"
              style="padding-left: {depth * 16 + 8}px;"
            >
              {#if node.is_dir}
                <button
                  class="w-4 h-4 flex items-center justify-center text-text-tertiary hover:text-text-secondary transition-colors shrink-0"
                  aria-label="Toggle folder"
                  onclick={() => toggleDir(node.path)}
                >
                  <svg class="w-3 h-3 transition-transform {isExpanded ? 'rotate-90' : ''}" viewBox="0 0 24 24" fill="currentColor">
                    <path d="M9 18l6-6-6-6" />
                  </svg>
                </button>
                <svg class="w-4 h-4 text-accent shrink-0" viewBox="0 0 24 24" fill="currentColor" opacity="0.8">
                  <path d="M10 4H4a2 2 0 00-2 2v12a2 2 0 002 2h16a2 2 0 002-2V8a2 2 0 00-2-2h-8l-2-2z" />
                </svg>
                <button class="text-text-primary truncate text-left bg-transparent border-none p-0 cursor-pointer" onclick={() => toggleDir(node.path)}>{node.name}</button>
                <span class="text-[11px] text-text-tertiary ml-1 shrink-0">{countFiles(node)} files</span>
              {:else}
                <div class="w-4 shrink-0"></div>
                <svg class="w-4 h-4 text-text-tertiary shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
                  <path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" />
                  <polyline points="14 2 14 8 20 8" />
                </svg>
                <span class="text-text-secondary truncate">{node.name}</span>
              {/if}

              <!-- Right-aligned size + download -->
              <span class="text-[11px] text-text-tertiary ml-auto mr-1 font-mono shrink-0">{formatBytes(node.is_dir ? calcFolderSize(node) : node.size)}</span>

              <!-- Download button (hidden for root-level folders) -->
              {#if !(node.is_dir && depth === 0)}
              <button
                class="opacity-0 group-hover:opacity-100 transition-opacity p-1 rounded hover:bg-accent-muted shrink-0 {isDownloading ? '!opacity-100' : ''}"
                onclick={() => node.is_dir ? downloadFolder(node) : downloadSingleFile(node)}
                disabled={isDownloading}
                title={node.is_dir ? "Download folder" : "Download file"}
                aria-label={node.is_dir ? "Download folder" : "Download file"}
              >
                {#if isDownloading}
                  <svg class="w-3.5 h-3.5 text-accent animate-spin" viewBox="0 0 24 24" fill="none">
                    <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="3" />
                    <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z" />
                  </svg>
                {:else}
                  <svg class="w-3.5 h-3.5 text-text-tertiary group-hover:text-accent transition-colors" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                    <path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4" />
                    <polyline points="7 10 12 15 17 10" />
                    <line x1="12" y1="15" x2="12" y2="3" />
                  </svg>
                {/if}
              </button>
              {/if}
            </div>

            {#if node.is_dir && isExpanded && node.children.length > 0}
              {#each node.children as child}
                {@render renderNode(child, depth + 1)}
              {/each}
            {/if}
          </div>
        {/snippet}

        {#if scanResult.files.length === 0}
          <p class="text-text-tertiary text-sm italic px-2 py-4">No files found in this archive.</p>
        {:else}
          {#each scanResult.files as rootNode}
            {@render renderNode(rootNode, 0)}
          {/each}
        {/if}
      </div>
    {:else if !isLoading}
      <!-- Empty state -->
      <div class="flex-1 flex flex-col items-center justify-center gap-3 opacity-40">
        <svg class="w-12 h-12 text-text-tertiary" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1">
          <path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4" />
          <polyline points="7 10 12 15 17 10" />
          <line x1="12" y1="15" x2="12" y2="3" />
        </svg>
        <p class="text-sm text-text-tertiary">Enter a remote ZIP URL to explore its contents</p>
      </div>
    {/if}
  </main>

  <!-- Success toast -->
  {#if toastMsg}
    <div class="absolute bottom-4 left-1/2 -translate-x-1/2 bg-bg-elevated border border-success/30 rounded-lg shadow-lg px-4 py-2.5 flex items-center gap-2 z-50 animate-fade-in">
      <svg class="w-4 h-4 text-success shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <path d="M20 6L9 17l-5-5" />
      </svg>
      <span class="text-sm text-text-primary">{toastMsg}</span>
      <button class="ml-2 text-text-tertiary hover:text-text-secondary text-xs" onclick={() => { toastMsg = ""; if (toastTimer) clearTimeout(toastTimer); }}>✕</button>
    </div>
  {/if}
</div>
