import { describe, expect, it } from "vitest";
import { hintsFor, keyLabel, SHORTCUT_GROUPS, type ShortcutState } from "./shortcuts";

const tree: ShortcutState = {
  context: "tree",
  hasArchive: true,
  searching: false,
  selectionCount: 0,
  focusedIsDir: true,
};

function labels(state: ShortcutState): string[] {
  return hintsFor(state).map((hint) => hint.label);
}

describe("hintsFor", () => {
  it("offers only scanning before an archive is open", () => {
    expect(labels({ ...tree, context: "url", hasArchive: false })).toEqual(["Scan"]);
  });

  it("adds search to the URL box once an archive is open", () => {
    expect(labels({ ...tree, context: "url" })).toEqual(["Scan", "Search files"]);
  });

  it("explains clearing and patterns in the search box", () => {
    expect(labels({ ...tree, context: "search" })).toEqual(["Clear search", "Any text", "One character"]);
  });

  it("names what Enter does for the focused row", () => {
    expect(labels(tree)).toContain("Open folder");
    expect(labels({ ...tree, focusedIsDir: false })).toContain("Download");
  });

  it("only offers clearing a selection when there is one", () => {
    expect(labels(tree)).not.toContain("Clear selection");
    expect(labels({ ...tree, selectionCount: 2 })).toContain("Clear selection");
  });

  it("drops tree-only keys in search results, where Enter shows the item in the tree", () => {
    const results = labels({ ...tree, searching: true });
    expect(results).toContain("Show in tree");
    expect(results).not.toContain("Collapse or expand");
    expect(results).not.toContain("Open folder");
  });

  it("puts moving and selecting first, since narrow windows cut off the end", () => {
    expect(labels(tree).slice(0, 3)).toEqual(["Move", "Collapse or expand", "Select"]);
  });
});

describe("keyLabel", () => {
  it("uses symbols on macOS and words elsewhere", () => {
    expect(keyLabel("mod", true).text).toBe("⌘");
    expect(keyLabel("mod", false).text).toBe("Ctrl");
    expect(keyLabel("enter", true).text).toBe("⏎");
    expect(keyLabel("enter", false).text).toBe("Enter");
  });

  it("gives symbols a spoken name", () => {
    expect(keyLabel("up", true)).toEqual({ text: "↑", name: "Up arrow" });
    expect(keyLabel("D", true)).toEqual({ text: "D", name: "D" });
  });
});

describe("SHORTCUT_GROUPS", () => {
  it("lists every hint's keys somewhere in the full list", () => {
    const listed = new Set(
      SHORTCUT_GROUPS.flatMap((group) => group.shortcuts).map((shortcut) => shortcut.keys.join("+")),
    );
    const states: ShortcutState[] = [
      tree,
      { ...tree, selectionCount: 1, focusedIsDir: false },
      { ...tree, searching: true, selectionCount: 1 },
      { ...tree, context: "url" },
      { ...tree, context: "search" },
    ];
    for (const state of states) {
      for (const hint of hintsFor(state)) expect(listed).toContain(hint.keys.join("+"));
    }
  });
});
