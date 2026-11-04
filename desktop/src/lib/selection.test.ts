import { describe, expect, it } from "vitest";
import {
  isSelected,
  partiallySelected,
  resolveSelection,
  selectRange,
  toggleSelected,
} from "./selection";
import { buildIndex } from "./tree";
import type { TreeNode } from "./types";

function node(parent: number | null, path: string, is_dir: boolean, size = 0): TreeNode {
  return {
    parent,
    name: path.split("/").pop()!,
    path,
    size,
    compressed_size: size,
    is_dir,
    file_count: 0,
    total_size: is_dir ? 0 : size,
  };
}

// 0 a/  1 a/b/  2 a/b/x.txt  3 a/b/y.txt  4 a/z.txt  5 c/  6 c/x.txt  7 top.txt
const index = buildIndex([
  node(null, "a", true),
  node(0, "a/b", true),
  node(1, "a/b/x.txt", false, 1),
  node(1, "a/b/y.txt", false, 2),
  node(0, "a/z.txt", false, 4),
  node(null, "c", true),
  node(5, "c/x.txt", false, 8),
  node(null, "top.txt", false, 16),
]);

describe("toggleSelected", () => {
  it("selects and deselects a single item", () => {
    const one = toggleSelected(index, new Set(), 2);
    expect([...one]).toEqual([2]);
    expect([...toggleSelected(index, one, 2)]).toEqual([]);
  });

  it("selecting a folder replaces selected items inside it", () => {
    const next = toggleSelected(index, new Set([2, 4, 7]), 0);
    expect([...next].sort()).toEqual([0, 7]);
  });

  it("deselecting inside a selected folder keeps everything else selected", () => {
    const next = toggleSelected(index, new Set([0]), 2);
    expect([...next].sort()).toEqual([3, 4]);
    expect(isSelected(index, next, 2)).toBe(false);
    expect(isSelected(index, next, 3)).toBe(true);
    expect(isSelected(index, next, 4)).toBe(true);
  });
});

describe("isSelected and partiallySelected", () => {
  it("treats everything under a selected folder as selected", () => {
    const selection = new Set([1]);
    expect(isSelected(index, selection, 2)).toBe(true);
    expect(isSelected(index, selection, 1)).toBe(true);
    expect(isSelected(index, selection, 0)).toBe(false);
    expect(isSelected(index, selection, 4)).toBe(false);
  });

  it("marks ancestors of selected items as partial", () => {
    expect([...partiallySelected(index, new Set([2]))].sort()).toEqual([0, 1]);
    expect([...partiallySelected(index, new Set([7]))]).toEqual([]);
  });
});

describe("selectRange", () => {
  const rows = [0, 1, 2, 3, 4, 5];

  it("selects inclusively in either direction, adding to the selection", () => {
    expect([...selectRange(new Set([7]), rows, 1, 3)].sort()).toEqual([1, 2, 3, 7]);
    expect([...selectRange(new Set(), rows, 3, 1)].sort()).toEqual([1, 2, 3]);
  });

  it("falls back to the target row when the anchor isn't visible", () => {
    expect([...selectRange(new Set(), rows, 99, 2)]).toEqual([2]);
  });
});

describe("resolveSelection", () => {
  it("keeps a single folder's name by using its parent as the base", () => {
    const resolved = resolveSelection(index, new Set([1]));
    expect(resolved.base).toBe("a");
    expect(resolved.files.sort()).toEqual(["a/b/x.txt", "a/b/y.txt"]);
    expect(resolved.bytes).toBe(3);
  });

  it("uses the archive root for a top-level folder", () => {
    expect(resolveSelection(index, new Set([5])).base).toBe("");
  });

  it("uses the closest common folder, so same-named files can't collide", () => {
    const resolved = resolveSelection(index, new Set([2, 4]));
    expect(resolved.base).toBe("a");
    expect(resolveSelection(index, new Set([2, 6])).base).toBe("");
  });

  it("uses the shared parent for siblings", () => {
    expect(resolveSelection(index, new Set([2, 3])).base).toBe("a/b");
  });

  it("doesn't count items covered by a selected folder twice", () => {
    const resolved = resolveSelection(index, new Set([0, 2]));
    expect(resolved.files.length).toBe(3);
    expect(resolved.bytes).toBe(7);
    expect(resolved.roots).toEqual([0]);
  });

  it("records each file's size", () => {
    expect(resolveSelection(index, new Set([7])).sizes.get("top.txt")).toBe(16);
  });
});
