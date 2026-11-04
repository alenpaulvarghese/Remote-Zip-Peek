export type ShortcutContext = "url" | "search" | "tree";

export interface Shortcut {
  /** Keys pressed together. See `keyLabel` for the special names. */
  keys: string[];
  label: string;
}

export interface ShortcutGroup {
  title: string;
  shortcuts: Shortcut[];
}

export interface ShortcutState {
  context: ShortcutContext;
  hasArchive: boolean;
  searching: boolean;
  selectionCount: number;
  focusedIsDir: boolean | null;
}

const MOVE: Shortcut = { keys: ["up", "down"], label: "Move" };
const SELECT: Shortcut = { keys: ["space"], label: "Select" };
const DOWNLOAD: Shortcut = { keys: ["D"], label: "Download item" };
const EXTEND: Shortcut = { keys: ["shift", "up", "down"], label: "Extend selection" };
const SELECT_ALL: Shortcut = { keys: ["mod", "A"], label: "Select all" };
const SEARCH: Shortcut = { keys: ["mod", "F"], label: "Search files" };
const SCAN: Shortcut = { keys: ["enter"], label: "Scan" };

/** The shortcuts that work right now, most useful first, since narrow windows show only the first few. */
export function hintsFor(state: ShortcutState): Shortcut[] {
  if (state.context === "url") {
    return state.hasArchive ? [SCAN, SEARCH] : [SCAN];
  }

  if (state.context === "search") {
    return [
      { keys: ["esc"], label: "Clear search" },
      { keys: ["*"], label: "Any text" },
      { keys: ["?"], label: "One character" },
    ];
  }

  const clearSelection: Shortcut[] =
    state.selectionCount > 0 ? [{ keys: ["esc"], label: "Clear selection" }] : [];
  if (state.searching) {
    return [
      MOVE,
      SELECT,
      ...clearSelection,
      { keys: ["enter"], label: "Show in tree" },
      DOWNLOAD,
      EXTEND,
      SELECT_ALL,
      SEARCH,
    ];
  }

  return [
    MOVE,
    { keys: ["left", "right"], label: "Collapse or expand" },
    SELECT,
    ...clearSelection,
    { keys: ["enter"], label: state.focusedIsDir === false ? "Download" : "Open folder" },
    DOWNLOAD,
    EXTEND,
    SELECT_ALL,
    SEARCH,
  ];
}

export const SHORTCUT_GROUPS: ShortcutGroup[] = [
  {
    title: "Move around",
    shortcuts: [
      MOVE,
      { keys: ["left", "right"], label: "Collapse or expand a folder" },
      { keys: ["PgUp", "PgDn"], label: "Move one page" },
      { keys: ["Home", "End"], label: "First or last row" },
    ],
  },
  {
    title: "Select",
    shortcuts: [
      { keys: ["space"], label: "Select or deselect" },
      EXTEND,
      { keys: ["mod", "A"], label: "Select all rows shown" },
      { keys: ["esc"], label: "Clear selection" },
      { keys: ["shift", "click"], label: "Select a range" },
      { keys: ["mod", "click"], label: "Add or remove an item" },
    ],
  },
  {
    title: "Open and download",
    shortcuts: [
      { keys: ["enter"], label: "Open folder or download file" },
      DOWNLOAD,
      { keys: ["enter"], label: "In search results: show in tree" },
    ],
  },
  {
    title: "Search and general",
    shortcuts: [
      SEARCH,
      { keys: ["esc"], label: "Clear search" },
      { keys: ["*"], label: "In a search: any text" },
      { keys: ["?"], label: "In a search: one character" },
      { keys: ["enter"], label: "In the URL box: scan" },
      { keys: ["?"], label: "Show or hide this list" },
    ],
  },
];

export interface KeyLabel {
  text: string;
  /** What a screen reader should say, when the text is a symbol. */
  name: string;
}

export function keyLabel(key: string, isMac: boolean): KeyLabel {
  const named = (text: string, name: string): KeyLabel => ({ text, name });
  switch (key) {
    case "mod":
      return isMac ? named("⌘", "Command") : named("Ctrl", "Control");
    case "shift":
      return isMac ? named("⇧", "Shift") : named("Shift", "Shift");
    case "enter":
      return isMac ? named("⏎", "Return") : named("Enter", "Enter");
    case "esc":
      return named("Esc", "Escape");
    case "space":
      return named("Space", "Space");
    case "click":
      return named("Click", "Click");
    case "up":
      return named("↑", "Up arrow");
    case "down":
      return named("↓", "Down arrow");
    case "left":
      return named("←", "Left arrow");
    case "right":
      return named("→", "Right arrow");
    default:
      return named(key, key);
  }
}
