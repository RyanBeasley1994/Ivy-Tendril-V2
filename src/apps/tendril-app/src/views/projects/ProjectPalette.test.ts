import { describe, expect, it } from "vitest";
import { fuzzyScore } from "./ProjectPalette";

describe("fuzzyScore", () => {
  it("matches a hyphenated project typed with a space", () => {
    // This used to match only the project's git repository, so ⌘K opened Git instead of the project.
    expect(fuzzyScore("ypf direct", "YPF-Direct propriotec")).not.toBeNull();
    expect(fuzzyScore("direct ypf", "YPF-Direct")).not.toBeNull();
    expect(fuzzyScore("ypf-direct", "YPF-Direct")).not.toBeNull();
  });

  it("still needs every word to match", () => {
    expect(fuzzyScore("ypf zebra", "YPF-Direct")).toBeNull();
    expect(fuzzyScore("", "anything")).toBe(0);
    expect(fuzzyScore("mono", "Monorepo-Propfirm")).not.toBeNull();
  });
});
