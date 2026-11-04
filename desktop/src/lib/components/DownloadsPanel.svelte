<script lang="ts">
  import { calcPercent, type DownloadJob, type DownloadManager } from "$lib/downloads.svelte";
  import { formatBytes, formatCount, plural } from "$lib/format";

  const MAX_FAILURES_SHOWN = 50;

  let { manager }: { manager: DownloadManager } = $props();
  let collapsed = $state(false);

  let hasFinished = $derived(manager.jobs.some((job) => job.status !== "running"));

  function statusText(job: DownloadJob): string {
    switch (job.status) {
      case "running":
        return job.bytesPerSecond > 0 ? `${formatBytes(job.bytesPerSecond)}/s` : "Starting…";
      case "done":
        return "Done";
      case "cancelled":
        return "Cancelled";
      case "failed":
        return `${formatCount(job.failures.length)} failed`;
    }
  }

  function basename(path: string): string {
    return path.split("/").filter(Boolean).pop() ?? path;
  }
</script>

{#if manager.jobs.length > 0}
  <section class="shrink-0 border-t border-border-default bg-bg-surface flex flex-col max-h-[30%] min-h-0" aria-label="Downloads">
    <div class="flex items-center gap-2 px-4 py-2 shrink-0">
      <button
        type="button"
        class="flex items-center gap-1.5 text-xs font-semibold text-text-primary"
        aria-expanded={!collapsed}
        onclick={() => (collapsed = !collapsed)}
      >
        <svg class="w-3 h-3 transition-transform {collapsed ? '' : 'rotate-90'}" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
          <path d="M9 18l6-6-6-6" />
        </svg>
        Downloads
        {#if manager.runningCount > 0}
          <span class="font-normal text-text-secondary">({manager.runningCount} active)</span>
        {/if}
      </button>
      {#if hasFinished}
        <button type="button" class="ml-auto text-xs text-text-tertiary hover:text-text-secondary" onclick={() => manager.clearFinished()}>
          Clear finished
        </button>
      {/if}
    </div>

    {#if !collapsed}
      <ul class="overflow-auto px-4 pb-3 space-y-2 min-h-0 list-none m-0">
        {#each manager.jobs as job (job.key)}
          {@const percent = calcPercent(job)}
          <li class="bg-bg-card border border-border-default rounded-lg px-3 py-2 text-xs">
            <div class="flex items-center gap-2">
              <span class="truncate min-w-0 font-medium text-text-primary" title={job.label}>{job.label}</span>
              <span class="ml-auto shrink-0 {job.status === 'failed' ? 'text-error' : job.status === 'done' ? 'text-success' : 'text-text-secondary'}">{statusText(job)}</span>
            </div>

            <div class="flex items-center gap-2 mt-1 text-text-secondary">
              <span>{formatBytes(job.doneBytes)} of {formatBytes(job.totalBytes)}</span>
              <span>·</span>
              <span>{formatCount(job.filesDone)} of {plural(job.totalFiles, "file")}</span>
              {#if job.status !== "running" && job.networkBytes > 0}
                <span>·</span>
                <span title="Data transferred over the network, including archive metadata">{formatBytes(job.networkBytes)} fetched</span>
              {/if}
            </div>

            {#if job.status === "running"}
              <div
                class="w-full h-1 bg-bg-elevated rounded-full overflow-hidden mt-1.5"
                role="progressbar"
                aria-label="Progress of {job.label}"
                aria-valuemin="0"
                aria-valuemax="100"
                aria-valuenow={Math.round(percent)}
              >
                <div class="h-full bg-accent rounded-full transition-[width] duration-200" style="width: {percent}%"></div>
              </div>

              {#if job.active.length > 0}
                <ul class="mt-1.5 space-y-0.5 list-none p-0 m-0">
                  {#each job.active as file (file.path)}
                    <li class="flex gap-2 text-[11px] text-text-tertiary">
                      <span class="truncate min-w-0" title={file.path}>{basename(file.path)}</span>
                      <span class="ml-auto shrink-0 font-mono">{formatBytes(file.done)} / {formatBytes(manager.sizeOf(job, file.path))}</span>
                    </li>
                  {/each}
                </ul>
              {/if}
            {/if}

            {#if job.failures.length > 0}
              <details class="mt-1.5">
                <summary class="cursor-pointer text-error">Show {plural(job.failures.length, "failure")}</summary>
                <ul class="mt-1 space-y-1 list-none p-0 m-0 select-text">
                  {#each job.failures.slice(0, MAX_FAILURES_SHOWN) as failure (failure.path)}
                    <li class="text-[11px] text-text-secondary break-words">
                      <span class="text-text-primary">{failure.path}</span>: {failure.error}
                    </li>
                  {/each}
                  {#if job.failures.length > MAX_FAILURES_SHOWN}
                    <li class="text-[11px] text-text-tertiary">and {formatCount(job.failures.length - MAX_FAILURES_SHOWN)} more</li>
                  {/if}
                </ul>
              </details>
            {/if}

            <div class="flex gap-3 mt-1.5 empty:hidden">
              {#if job.status === "running"}
                <button type="button" class="text-text-secondary hover:text-text-primary disabled:opacity-40" disabled={job.jobId === null} aria-label="Cancel {job.label}" onclick={() => manager.cancel(job)}>Cancel</button>
              {:else}
                {#if job.failures.length > 0 && job.status === "failed"}
                  <button type="button" class="text-accent hover:text-accent-hover" onclick={() => manager.retryFailed(job)}>Retry {formatCount(job.failures.length)} failed</button>
                {/if}
                {#if job.filesDone > 0}
                  <button type="button" class="text-accent hover:text-accent-hover" onclick={() => manager.reveal(job)}>Show in folder</button>
                {/if}
                <button type="button" class="text-text-tertiary hover:text-text-secondary ml-auto" aria-label="Remove {job.label} from the list" onclick={() => manager.remove(job)}>Remove</button>
              {/if}
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </section>
{/if}
