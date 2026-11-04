import { ancestorsOf, nodeSize, type TreeIndex } from "./tree";

/** Selected node IDs. Selecting a folder implies everything under it. */
export type Selection = ReadonlySet<number>;

export function isSelected(index: TreeIndex, selection: Selection, id: number): boolean {
  if (selection.size === 0) return false;
  for (let current: number | null = id; current !== null; current = index.nodes[current].parent) {
    if (selection.has(current)) return true;
  }
  return false;
}

/** Folders that contain a selected item but aren't fully selected themselves. */
export function partiallySelected(index: TreeIndex, selection: Selection): Set<number> {
  const partial = new Set<number>();
  for (const id of selection) {
    for (let p = index.nodes[id].parent; p !== null && !partial.has(p); p = index.nodes[p].parent) {
      partial.add(p);
    }
  }
  return partial;
}

export function toggleSelected(index: TreeIndex, selection: Selection, id: number): Set<number> {
  const next = new Set(selection);
  if (next.has(id)) {
    next.delete(id);
    return next;
  }

  const ancestors = ancestorsOf(index, id);
  const selectedFrom = ancestors.findIndex((ancestor) => next.has(ancestor));
  if (selectedFrom < 0) {
    // Selecting a folder replaces any individually selected items inside it.
    if (index.nodes[id].is_dir) removeDescendants(index, next, id);
    next.add(id);
    return next;
  }

  // Deselecting something inside a selected folder: keep everything else in that folder selected.
  const path = [...ancestors.slice(selectedFrom), id];
  next.delete(path[0]);
  for (let level = 0; level + 1 < path.length; level++) {
    for (const sibling of index.children[path[level]] ?? []) {
      if (sibling !== path[level + 1]) next.add(sibling);
    }
  }
  return next;
}

function removeDescendants(index: TreeIndex, selection: Set<number>, id: number): void {
  if (selection.size === 0) return;
  const stack = [...(index.children[id] ?? [])];
  while (stack.length > 0) {
    const current = stack.pop()!;
    selection.delete(current);
    const children = index.children[current];
    if (children) for (const child of children) stack.push(child);
  }
}

/** Selects every row between two rows, adding to the current selection. */
export function selectRange(
  selection: Selection,
  rows: readonly number[],
  from: number,
  to: number,
): Set<number> {
  const next = new Set(selection);
  const a = rows.indexOf(from);
  const b = rows.indexOf(to);
  if (b < 0) return next;
  const start = a < 0 ? b : Math.min(a, b);
  const end = a < 0 ? b : Math.max(a, b);
  for (let i = start; i <= end; i++) next.add(rows[i]);
  return next;
}

export interface ResolvedSelection {
  /** Directory in the archive that destinations are relative to. Empty for the archive's root. */
  base: string;
  files: string[];
  bytes: number;
  sizes: Map<string, number>;
  /** The selected items, ignoring anything already covered by a selected folder. */
  roots: number[];
}

/** Expands a selection to its files, and finds the closest folder containing all of them. */
export function resolveSelection(index: TreeIndex, selection: Selection): ResolvedSelection {
  const { nodes, children } = index;
  const roots = [...selection].filter((id) => {
    for (let p = nodes[id].parent; p !== null; p = nodes[p].parent) {
      if (selection.has(p)) return false;
    }
    return true;
  });

  const files: string[] = [];
  const sizes = new Map<string, number>();
  let bytes = 0;
  const stack = [...roots];
  while (stack.length > 0) {
    const id = stack.pop()!;
    const node = nodes[id];
    if (node.is_dir) {
      const kids = children[id];
      if (kids) for (const child of kids) stack.push(child);
    } else {
      files.push(node.path);
      sizes.set(node.path, nodeSize(node));
      bytes += nodeSize(node);
    }
  }

  return { base: commonBase(index, roots), files, bytes, sizes, roots };
}

function commonBase(index: TreeIndex, roots: number[]): string {
  let common: number[] | null = null;
  for (const id of roots) {
    const ancestors = ancestorsOf(index, id);
    if (common === null) {
      common = ancestors;
      continue;
    }
    let shared = 0;
    while (shared < common.length && common[shared] === ancestors[shared]) shared++;
    common = common.slice(0, shared);
    if (common.length === 0) break;
  }
  return common && common.length > 0 ? index.nodes[common[common.length - 1]].path : "";
}
