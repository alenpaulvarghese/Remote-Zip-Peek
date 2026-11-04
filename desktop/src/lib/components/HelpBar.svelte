<script lang="ts">
  import {
    hintsFor,
    keyLabel,
    SHORTCUT_GROUPS,
    type Shortcut,
    type ShortcutState,
  } from "$lib/shortcuts";

  let { state: shortcutState, open = $bindable(false) }: { state: ShortcutState; open?: boolean } =
    $props();

  const isMac = /Mac|iPhone|iPad/.test(navigator.userAgent);

  let hints = $derived(hintsFor(shortcutState));

  function spokenKeys(shortcut: Shortcut): string {
    return shortcut.keys.map((key) => keyLabel(key, isMac).name).join(" ");
  }
</script>

{#snippet keys(shortcut: Shortcut)}
  <span class="inline-flex items-center gap-0.5 shrink-0" aria-label={spokenKeys(shortcut)} role="img">
    {#each shortcut.keys as key}
      <kbd class="inline-flex items-center justify-center min-w-[18px] h-[18px] px-1 rounded bg-bg-elevated border border-border-hover text-text-primary text-[10px] leading-none font-sans">{keyLabel(key, isMac).text}</kbd>
    {/each}
  </span>
{/snippet}

<aside class="relative shrink-0" aria-label="Keyboard shortcuts">
  {#if open}
    <div
      id="all-shortcuts"
      class="absolute bottom-full inset-x-0 z-40 max-h-[70vh] overflow-auto bg-bg-elevated border-t border-border-hover shadow-lg px-4 py-3 grid grid-cols-2 sm:grid-cols-4 gap-x-6 gap-y-3"
    >
      {#each SHORTCUT_GROUPS as group (group.title)}
        <section>
          <h2 class="m-0 mb-1.5 text-[10px] uppercase tracking-widest text-text-secondary font-medium">{group.title}</h2>
          <ul class="list-none m-0 p-0 space-y-1.5">
            {#each group.shortcuts as shortcut (shortcut.label)}
              <li class="flex items-start gap-2 text-[11px] text-text-secondary">
                {@render keys(shortcut)}
                <span class="pt-px">{shortcut.label}</span>
              </li>
            {/each}
          </ul>
        </section>
      {/each}
    </div>
  {/if}

  <div class="flex items-center gap-3 h-7 px-4 bg-bg-surface border-t border-border-default">
    <!-- Hints that don't fit wrap onto a hidden line, so a chip is never cut in half. -->
    <ul class="flex flex-wrap items-center gap-x-3 h-7 min-w-0 flex-1 overflow-hidden list-none m-0 p-0" data-testid="shortcut-hints">
      {#each hints as hint (hint.label)}
        <li class="flex items-center gap-1.5 h-7 text-[11px] text-text-secondary whitespace-nowrap">
          {@render keys(hint)}
          {hint.label}
        </li>
      {/each}
    </ul>

    <button
      type="button"
      class="flex items-center gap-1.5 shrink-0 text-[11px] text-text-secondary hover:text-text-primary whitespace-nowrap"
      aria-expanded={open}
      aria-controls="all-shortcuts"
      onclick={() => (open = !open)}
    >
      <kbd class="inline-flex items-center justify-center min-w-[18px] h-[18px] px-1 rounded bg-bg-elevated border border-border-hover text-text-primary text-[10px] leading-none font-sans">?</kbd>
      {open ? "Hide shortcuts" : "All shortcuts"}
    </button>
  </div>
</aside>
