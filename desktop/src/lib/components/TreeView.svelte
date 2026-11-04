<script lang="ts">
  import { formatBytes, plural } from "$lib/format";
  import { isSelected, type Selection } from "$lib/selection";
  import {
    nodeSize,
    type SearchMatcher,
    type TreeIndex,
    type TreeSorter,
  } from "$lib/tree";

  const ROW_HEIGHT = 28;
  const OVERSCAN = 10;

  interface Props {
    index: TreeIndex;
    sorter: TreeSorter;
    rows: number[];
    matcher: SearchMatcher | null;
    expanded: ReadonlySet<number>;
    selection: Selection;
    partial: ReadonlySet<number>;
    focusId: number | null;
    onFocus: (id: number) => void;
    onToggleExpanded: (id: number) => void;
    onToggleSelected: (id: number) => void;
    onSelectRange: (id: number) => void;
    onSelectAll: () => void;
    onClearSelection: () => void;
    onActivate: (id: number) => void;
    onDownload: (id: number) => void;
  }

  let {
    index,
    sorter,
    rows,
    matcher,
    expanded,
    selection,
    partial,
    focusId,
    onFocus,
    onToggleExpanded,
    onToggleSelected,
    onSelectRange,
    onSelectAll,
    onClearSelection,
    onActivate,
    onDownload,
  }: Props = $props();

  let viewport: HTMLDivElement;
  let scrollTop = $state(0);
  let viewportHeight = $state(0);

  let start = $derived(Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN));
  let end = $derived(
    Math.min(rows.length, Math.ceil((scrollTop + viewportHeight) / ROW_HEIGHT) + OVERSCAN),
  );
  let windowRows = $derived(rows.slice(start, end));
  let focusPosition = $derived(focusId === null ? -1 : rows.indexOf(focusId));

  // Keep the focused row on screen when it moves or the rows change.
  $effect(() => {
    if (focusPosition < 0 || !viewport) return;
    const top = focusPosition * ROW_HEIGHT;
    if (top < viewport.scrollTop) {
      viewport.scrollTop = top;
    } else if (top + ROW_HEIGHT > viewport.scrollTop + viewport.clientHeight) {
      viewport.scrollTop = top + ROW_HEIGHT - viewport.clientHeight;
    }
  });

  function moveFocus(position: number, extendSelection: boolean) {
    if (rows.length === 0) return;
    const id = rows[Math.max(0, Math.min(rows.length - 1, position))];
    onFocus(id);
    if (extendSelection) onSelectRange(id);
  }

  function handleKeydown(event: KeyboardEvent) {
    const id = focusId;
    const current = focusPosition;
    const page = Math.max(1, Math.floor(viewportHeight / ROW_HEIGHT) - 1);

    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "a") {
      onSelectAll();
    } else {
      switch (event.key) {
        case "ArrowDown":
          moveFocus(current + 1, event.shiftKey);
          break;
        case "ArrowUp":
          moveFocus(current < 0 ? 0 : current - 1, event.shiftKey);
          break;
        case "PageDown":
          moveFocus(current + page, event.shiftKey);
          break;
        case "PageUp":
          moveFocus(current - page, event.shiftKey);
          break;
        case "Home":
          moveFocus(0, event.shiftKey);
          break;
        case "End":
          moveFocus(rows.length - 1, event.shiftKey);
          break;
        case "ArrowRight":
          if (id !== null && !matcher && index.nodes[id].is_dir) {
            if (!expanded.has(id)) onToggleExpanded(id);
            else moveFocus(current + 1, false);
          }
          break;
        case "ArrowLeft":
          if (id !== null && !matcher) {
            const parent = index.nodes[id].parent;
            if (index.nodes[id].is_dir && expanded.has(id)) onToggleExpanded(id);
            else if (parent !== null) onFocus(parent);
          }
          break;
        case " ":
          if (id !== null) onToggleSelected(id);
          break;
        case "Enter":
          if (id !== null) onActivate(id);
          break;
        case "d":
        case "D":
          if (id !== null) onDownload(id);
          break;
        case "Escape":
          onClearSelection();
          break;
        default:
          return;
      }
    }
    event.preventDefault();
  }

  function handleRowClick(event: MouseEvent, id: number) {
    onFocus(id);
    if (event.shiftKey) onSelectRange(id);
    else if (event.metaKey || event.ctrlKey) onToggleSelected(id);
    else if (index.nodes[id].is_dir) onActivate(id);
    viewport.focus({ preventScroll: true });
  }

  function highlightParts(name: string): [string, string, string] | null {
    const range = matcher?.highlight(name);
    return range ? [name.slice(0, range[0]), name.slice(range[0], range[1]), name.slice(range[1])] : null;
  }

  function parentPath(id: number): string {
    const parent = index.nodes[id].parent;
    return parent === null ? "" : index.nodes[parent].path;
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div
  bind:this={viewport}
  bind:clientHeight={viewportHeight}
  class="tree-viewport flex-1 min-h-0 overflow-auto px-4 outline-none select-none"
  role="tree"
  aria-label="Archive contents"
  aria-multiselectable="true"
  aria-activedescendant={focusId !== null && focusPosition >= 0 ? `row-${focusId}` : undefined}
  tabindex="0"
  onscroll={() => (scrollTop = viewport.scrollTop)}
  onkeydown={handleKeydown}
  onfocus={() => {
    if (focusId === null || focusPosition < 0) moveFocus(Math.floor(scrollTop / ROW_HEIGHT), false);
  }}
>
  <div class="relative" style="height: {rows.length * ROW_HEIGHT}px;">
    {#each windowRows as id, i (id)}
      {@const node = index.nodes[id]}
      {@const selected = isSelected(index, selection, id)}
      {@const indeterminate = !selected && partial.has(id)}
      {@const isFocused = focusId === id}
      {@const isExpanded = expanded.has(id)}
      {@const parts = highlightParts(node.name)}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div
        id="row-{id}"
        role="treeitem"
        tabindex="-1"
        aria-level={matcher ? 1 : index.depth[id] + 1}
        aria-setsize={matcher ? rows.length : sorter.siblingCount(id)}
        aria-posinset={matcher ? start + i + 1 : sorter.position[id] + 1}
        aria-selected={selected}
        aria-expanded={node.is_dir && !matcher ? isExpanded : undefined}
        class="group absolute inset-x-0 flex items-center gap-1.5 pr-2 rounded-md text-sm cursor-default
          {selected ? 'bg-accent-muted' : 'hover:bg-bg-hover/60'}
          {isFocused ? 'tree-row-focused' : ''}"
        style="top: {(start + i) * ROW_HEIGHT}px; height: {ROW_HEIGHT}px; padding-left: {(matcher ? 0 : index.depth[id]) * 16 + 8}px;"
        onclick={(event) => handleRowClick(event, id)}
      >
        <input
          type="checkbox"
          class="shrink-0 accent-accent"
          tabindex="-1"
          aria-label="Select {node.name}"
          checked={selected}
          indeterminate={indeterminate}
          onclick={(event) => {
            event.stopPropagation();
            onFocus(id);
            if (event.shiftKey) onSelectRange(id);
            else onToggleSelected(id);
            viewport.focus({ preventScroll: true });
          }}
        />

        {#if node.is_dir && !matcher}
          <span class="w-4 h-4 flex items-center justify-center text-text-tertiary shrink-0" aria-hidden="true">
            <svg class="w-3 h-3 transition-transform {isExpanded ? 'rotate-90' : ''}" viewBox="0 0 24 24" fill="currentColor">
              <path d="M9 18l6-6-6-6" />
            </svg>
          </span>
        {:else if !matcher}
          <span class="w-4 shrink-0"></span>
        {/if}

        {#if node.is_dir}
          <svg class="w-4 h-4 text-accent shrink-0" viewBox="0 0 24 24" fill="currentColor" opacity="0.8" aria-hidden="true">
            <path d="M10 4H4a2 2 0 00-2 2v12a2 2 0 002 2h16a2 2 0 002-2V8a2 2 0 00-2-2h-8l-2-2z" />
          </svg>
        {:else}
          <svg class="w-4 h-4 text-text-tertiary shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true">
            <path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" />
            <polyline points="14 2 14 8 20 8" />
          </svg>
        {/if}

        <span class="truncate min-w-0 {node.is_dir ? 'text-text-primary' : 'text-text-secondary'}">
          {#if parts}{parts[0]}<mark class="bg-accent-muted text-text-accent rounded-sm">{parts[1]}</mark>{parts[2]}{:else}{node.name}{/if}
        </span>

        {#if node.is_dir}
          <span class="text-[11px] text-text-tertiary ml-1 shrink-0">{plural(node.file_count, "file")}</span>
        {/if}
        {#if matcher && parentPath(id)}
          <span class="text-[11px] text-text-tertiary ml-1 truncate min-w-0">{parentPath(id)}</span>
        {/if}

        <span class="text-[11px] text-text-tertiary ml-auto mr-1 pl-2 font-mono shrink-0">{formatBytes(nodeSize(node))}</span>

        <button
          type="button"
          tabindex="-1"
          class="p-1 rounded hover:bg-accent-muted shrink-0 opacity-50 group-hover:opacity-100 {isFocused ? '!opacity-100' : ''}"
          title="Download {node.name}"
          aria-label="Download {node.name}"
          onclick={(event) => {
            event.stopPropagation();
            onFocus(id);
            onDownload(id);
          }}
        >
          <svg class="w-3.5 h-3.5 text-text-tertiary group-hover:text-accent transition-colors" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true">
            <path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4" />
            <polyline points="7 10 12 15 17 10" />
            <line x1="12" y1="15" x2="12" y2="3" />
          </svg>
        </button>
      </div>
    {/each}
  </div>
</div>

<style>
  .tree-viewport:focus-visible :global(.tree-row-focused) {
    outline: 2px solid var(--color-accent);
    outline-offset: -2px;
  }
  :global(.tree-row-focused) {
    outline: 1px solid var(--color-border-hover);
    outline-offset: -1px;
  }
</style>
