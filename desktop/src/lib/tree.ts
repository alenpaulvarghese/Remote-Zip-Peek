import type { TreeNode } from "./types";

export type SortKey = "name" | "size" | "compressed" | "ratio";

export interface SortOrder {
  key: SortKey;
  descending: boolean;
}

export const NAME_ASCENDING: SortOrder = { key: "name", descending: false };

export interface TreeIndex {
  nodes: TreeNode[];
  roots: number[];
  /** Child IDs by parent ID, undefined for nodes with no children. */
  children: (number[] | undefined)[];
  depth: Uint16Array;
  /** Compressed size of all files at or under each node. */
  compressedTotal: Float64Array;
}

export function buildIndex(nodes: TreeNode[]): TreeIndex {
  const roots: number[] = [];
  const children: (number[] | undefined)[] = new Array(nodes.length);
  const depth = new Uint16Array(nodes.length);
  const compressedTotal = new Float64Array(nodes.length);

  for (let id = 0; id < nodes.length; id++) {
    const parent = nodes[id].parent;
    if (parent === null) {
      roots.push(id);
    } else {
      depth[id] = depth[parent] + 1;
      (children[parent] ??= []).push(id);
    }
  }

  // Parents precede children, so a reverse pass rolls totals up.
  for (let id = nodes.length - 1; id >= 0; id--) {
    compressedTotal[id] += nodes[id].compressed_size;
    const parent = nodes[id].parent;
    if (parent !== null) compressedTotal[parent] += compressedTotal[id];
  }

  return { nodes, roots, children, depth, compressedTotal };
}

export function nodeSize(node: TreeNode): number {
  return node.is_dir ? node.total_size : node.size;
}

function compareBy(index: TreeIndex, order: SortOrder): (a: number, b: number) => number {
  const { nodes, compressedTotal } = index;
  const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });
  const byName = (a: number, b: number) => collator.compare(nodes[a].name, nodes[b].name);
  const ratio = (id: number) => {
    const size = nodeSize(nodes[id]);
    return size === 0 ? 0 : compressedTotal[id] / size;
  };
  const metric: Record<SortKey, ((id: number) => number) | null> = {
    name: null,
    size: (id) => nodeSize(nodes[id]),
    compressed: (id) => compressedTotal[id],
    ratio,
  };
  const value = metric[order.key];
  const sign = order.descending ? -1 : 1;

  return (a, b) => {
    // Folders stay above files in either direction.
    if (nodes[a].is_dir !== nodes[b].is_dir) return nodes[a].is_dir ? -1 : 1;
    const difference = value ? value(a) - value(b) : byName(a, b);
    return difference !== 0 ? sign * difference : byName(a, b);
  };
}

export function sortIds(index: TreeIndex, ids: number[], order: SortOrder): number[] {
  return [...ids].sort(compareBy(index, order));
}

/** Sorts each folder once, when it first becomes visible. */
export class TreeSorter {
  private sorted = new Map<number, number[]>();
  private compare: (a: number, b: number) => number;
  /** Each node's position among its siblings, valid once the parent has been sorted. */
  readonly position: Uint32Array;

  constructor(
    private index: TreeIndex,
    order: SortOrder,
  ) {
    this.compare = compareBy(index, order);
    this.position = new Uint32Array(index.nodes.length);
  }

  private sortGroup(key: number, ids: number[]): number[] {
    let sorted = this.sorted.get(key);
    if (!sorted) {
      sorted = [...ids].sort(this.compare);
      sorted.forEach((id, i) => (this.position[id] = i));
      this.sorted.set(key, sorted);
    }
    return sorted;
  }

  roots(): number[] {
    return this.sortGroup(-1, this.index.roots);
  }

  childrenOf(id: number): number[] {
    return this.sortGroup(id, this.index.children[id] ?? []);
  }

  siblingCount(id: number): number {
    const parent = this.index.nodes[id].parent;
    return parent === null ? this.index.roots.length : (this.index.children[parent]?.length ?? 0);
  }
}

/** Lists the rows on screen: roots, plus the children of every expanded folder, in order. */
export function visibleRows(sorter: TreeSorter, expanded: ReadonlySet<number>): number[] {
  const rows: number[] = [];
  const stack = [...sorter.roots()].reverse();
  while (stack.length > 0) {
    const id = stack.pop()!;
    rows.push(id);
    if (expanded.has(id)) {
      const children = sorter.childrenOf(id);
      for (let i = children.length - 1; i >= 0; i--) stack.push(children[i]);
    }
  }
  return rows;
}

export interface SearchMatcher {
  matches(path: string): boolean;
  /** Where the match falls in a name, for highlighting. Glob patterns aren't highlighted. */
  highlight(name: string): [start: number, end: number] | null;
}

/** Plain text matches anywhere in the path, ignoring case. `*` and `?` make it a glob. */
export function createMatcher(query: string): SearchMatcher | null {
  const trimmed = query.trim().toLowerCase();
  if (!trimmed) return null;

  if (/[*?]/.test(trimmed)) {
    const source = trimmed
      .replace(/[.+^${}()|[\]\\]/g, "\\$&")
      .replace(/\*/g, "[^]*?")
      .replace(/\?/g, "[^]");
    const pattern = new RegExp(`(?:^|/)${source}$`);
    return { matches: (path) => pattern.test(path.toLowerCase()), highlight: () => null };
  }

  return {
    matches: (path) => path.toLowerCase().includes(trimmed),
    highlight: (name) => {
      const start = name.toLowerCase().indexOf(trimmed);
      return start < 0 ? null : [start, start + trimmed.length];
    },
  };
}

export function searchRows(index: TreeIndex, matcher: SearchMatcher, order: SortOrder): number[] {
  const matches: number[] = [];
  for (let id = 0; id < index.nodes.length; id++) {
    if (matcher.matches(index.nodes[id].path)) matches.push(id);
  }
  return sortIds(index, matches, order);
}

/** A node's ancestors, outermost first, not including the node itself. */
export function ancestorsOf(index: TreeIndex, id: number): number[] {
  const ancestors: number[] = [];
  for (let p = index.nodes[id].parent; p !== null; p = index.nodes[p].parent) ancestors.push(p);
  return ancestors.reverse();
}

export function allFolderIds(index: TreeIndex): number[] {
  const ids: number[] = [];
  index.nodes.forEach((node, id) => {
    if (node.is_dir) ids.push(id);
  });
  return ids;
}
