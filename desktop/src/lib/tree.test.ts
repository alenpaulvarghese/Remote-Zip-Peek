import { describe, expect, it } from "vitest";
import {
  ancestorsOf,
  buildIndex,
  createMatcher,
  searchRows,
  sortIds,
  TreeSorter,
  visibleRows,
  NAME_ASCENDING,
} from "./tree";
import type { TreeNode } from "./types";

function node(parent: number | null, path: string, is_dir: boolean, size = 0, compressed = 0): TreeNode {
  return {
    parent,
    name: path.split("/").pop()!,
    path,
    size,
    compressed_size: compressed,
    is_dir,
    file_count: is_dir ? 0 : 1,
    total_size: size,
  };
}

// 0 data/  1 data/b.csv  2 data/sub/  3 data/sub/a.json  4 data/a10.csv  5 data/a2.csv  6 readme.md
const nodes = [
  { ...node(null, "data", true), total_size: 111 },
  node(0, "data/b.csv", false, 100, 90),
  { ...node(0, "data/sub", true), total_size: 8 },
  node(2, "data/sub/a.json", false, 8, 2),
  node(0, "data/a10.csv", false, 1, 1),
  node(0, "data/a2.csv", false, 2, 1),
  node(null, "readme.md", false, 50, 5),
];
const index = buildIndex(nodes);

describe("buildIndex", () => {
  it("links roots, children and depth", () => {
    expect(index.roots).toEqual([0, 6]);
    expect(index.children[0]).toEqual([1, 2, 4, 5]);
    expect(index.children[6]).toBeUndefined();
    expect(index.depth[3]).toBe(2);
  });

  it("rolls compressed sizes up to folders", () => {
    expect(index.compressedTotal[2]).toBe(2);
    expect(index.compressedTotal[0]).toBe(94);
  });
});

describe("visibleRows", () => {
  const sorter = () => new TreeSorter(index, NAME_ASCENDING);

  it("shows only roots when nothing is expanded", () => {
    expect(visibleRows(sorter(), new Set())).toEqual([0, 6]);
  });

  it("shows folders first, then natural name order", () => {
    expect(visibleRows(sorter(), new Set([0]))).toEqual([0, 2, 5, 4, 1, 6]);
  });

  it("nests expanded children under their parent", () => {
    expect(visibleRows(sorter(), new Set([0, 2]))).toEqual([0, 2, 3, 5, 4, 1, 6]);
  });

  it("hides children of a collapsed ancestor even if they are expanded", () => {
    expect(visibleRows(sorter(), new Set([2]))).toEqual([0, 6]);
  });

  it("records each row's position among its siblings", () => {
    const s = sorter();
    visibleRows(s, new Set([0]));
    expect(s.position[2]).toBe(0);
    expect(s.position[1]).toBe(3);
    expect(s.siblingCount(1)).toBe(4);
    expect(s.siblingCount(6)).toBe(2);
  });
});

describe("sortIds", () => {
  it("sorts by size in either direction, keeping folders first", () => {
    expect(sortIds(index, [1, 2, 4, 5], { key: "size", descending: true })).toEqual([2, 1, 5, 4]);
    expect(sortIds(index, [1, 2, 4, 5], { key: "size", descending: false })).toEqual([2, 4, 5, 1]);
  });

  it("sorts by compression ratio", () => {
    expect(sortIds(index, [1, 4, 5], { key: "ratio", descending: false })).toEqual([5, 1, 4]);
  });
});

describe("createMatcher", () => {
  it("returns null for a blank query", () => {
    expect(createMatcher("   ")).toBeNull();
  });

  it("matches plain text anywhere in the path, ignoring case", () => {
    const matcher = createMatcher("SUB")!;
    expect(matcher.matches("data/sub/a.json")).toBe(true);
    expect(matcher.matches("data/b.csv")).toBe(false);
    expect(matcher.highlight("my-Sub.txt")).toEqual([3, 6]);
  });

  it("treats * and ? as a glob over the whole path or a trailing part of it", () => {
    const json = createMatcher("*.json")!;
    expect(json.matches("data/sub/a.json")).toBe(true);
    expect(json.matches("data/sub/a.json.bak")).toBe(false);
    const nested = createMatcher("sub/*.json")!;
    expect(nested.matches("data/sub/a.json")).toBe(true);
    expect(nested.matches("datasub/a.json")).toBe(false);
    const single = createMatcher("a?.csv")!;
    expect(single.matches("data/a2.csv")).toBe(true);
    expect(single.matches("data/a10.csv")).toBe(false);
    expect(json.highlight("a.json")).toBeNull();
  });

  it("treats regular expression characters literally", () => {
    const matcher = createMatcher("a+(b)*")!;
    expect(matcher.matches("a+(b)c")).toBe(true);
    expect(matcher.matches("aab")).toBe(false);
  });
});

describe("searchRows", () => {
  it("returns sorted matches from anywhere in the tree", () => {
    expect(searchRows(index, createMatcher("*.csv")!, NAME_ASCENDING)).toEqual([5, 4, 1]);
    expect(searchRows(index, createMatcher("sub")!, NAME_ASCENDING)).toEqual([2, 3]);
    expect(searchRows(index, createMatcher("nothing")!, NAME_ASCENDING)).toEqual([]);
  });
});

describe("ancestorsOf", () => {
  it("lists ancestors outermost first", () => {
    expect(ancestorsOf(index, 3)).toEqual([0, 2]);
    expect(ancestorsOf(index, 0)).toEqual([]);
  });
});

describe("large trees", () => {
  it("handles 300,000 files in one folder", () => {
    const big: TreeNode[] = [node(null, "d", true)];
    for (let i = 0; i < 300_000; i++) big.push(node(0, `d/file-${i}.bin`, false, i, i));
    const bigIndex = buildIndex(big);
    const rows = visibleRows(new TreeSorter(bigIndex, NAME_ASCENDING), new Set([0]));
    expect(rows.length).toBe(300_001);
    expect(big[rows[1]].name).toBe("file-0.bin");
    expect(big[rows[300_000]].name).toBe("file-299999.bin");
    expect(searchRows(bigIndex, createMatcher("file-29999")!, NAME_ASCENDING).length).toBe(11);
  });
});
