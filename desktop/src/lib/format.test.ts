import { describe, expect, it } from "vitest";
import { calcEfficiency, formatBytes, normalizeWorkers, plural } from "./format";

describe("normalizeWorkers", () => {
  it("falls back to the default for an empty field", () => {
    expect(normalizeWorkers(null)).toBe(10);
    expect(normalizeWorkers(undefined)).toBe(10);
    expect(normalizeWorkers(NaN)).toBe(10);
  });

  it("keeps values within the limits", () => {
    expect(normalizeWorkers(0)).toBe(1);
    expect(normalizeWorkers(-5)).toBe(1);
    expect(normalizeWorkers(500)).toBe(20);
    expect(normalizeWorkers(7)).toBe(7);
    expect(normalizeWorkers(2.6)).toBe(3);
  });
});

describe("calcEfficiency", () => {
  it("reports the share not fetched", () => {
    expect(calcEfficiency(1000, 100)).toBe("90.0");
  });

  it("never reports negative savings", () => {
    expect(calcEfficiency(100, 500)).toBe("0.0");
    expect(calcEfficiency(0, 0)).toBe("0");
  });
});

describe("formatBytes and plural", () => {
  it("formats sizes", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(1536)).toBe("1.5 KB");
  });

  it("handles singular and irregular plurals", () => {
    expect(plural(1, "file")).toBe("1 file");
    expect(plural(1000, "file")).toBe("1,000 files");
    expect(plural(2, "match", "matches")).toBe("2 matches");
  });
});
