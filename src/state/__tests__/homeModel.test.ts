import { describe, expect, it } from "vitest";
import {
  baseName,
  displayFolder,
  formatBytes,
  formatDate,
  frequentFolders,
  groupRecent,
  parentFolder,
  type RecentFile,
} from "../homeModel";

const NOW_MS = Date.UTC(2026, 8, 23, 3, 0, 0);
const DAY = 86_400;

function file(path: string, daysAgo: number): RecentFile {
  return {
    path,
    name: baseName(path),
    last_opened: NOW_MS / 1000 - daysAgo * DAY,
    pinned: false,
    available: true,
    cover: null,
    size: 1000,
    modified: NOW_MS / 1000 - 100 * DAY,
  };
}

describe("groupRecent", () => {
  it("splits at thirty days by last opened, keeping order", () => {
    const groups = groupRecent(
      [file("C:\\a.pdf", 0), file("C:\\b.pdf", 29.9), file("C:\\c.pdf", 30.1), file("C:\\d.pdf", 400)],
      NOW_MS,
    );
    expect(groups.map((g) => g.key)).toEqual(["last30", "earlier"]);
    expect(groups[0]?.files.map((f) => f.name)).toEqual(["a.pdf", "b.pdf"]);
    expect(groups[1]?.files.map((f) => f.name)).toEqual(["c.pdf", "d.pdf"]);
  });

  it("reads timestamps as seconds, which is what the store writes", () => {
    // Were they read as milliseconds, a file opened an hour ago would be
    // dated to January 1970 and land in "earlier".
    const [only] = groupRecent([file("C:\\a.pdf", 1 / 24)], NOW_MS);
    expect(only?.key).toBe("last30");
  });

  it("drops empty groups and files never opened go to earlier", () => {
    expect(groupRecent([], NOW_MS)).toEqual([]);
    const never = { ...file("C:\\x.pdf", 0), last_opened: null };
    expect(groupRecent([never], NOW_MS).map((g) => g.key)).toEqual(["earlier"]);
  });
});

describe("paths", () => {
  it("finds the parent for both separators and keeps a drive root readable", () => {
    expect(parentFolder("C:\\Users\\a\\Documents\\x.pdf")).toBe("C:\\Users\\a\\Documents");
    expect(parentFolder("C:\\x.pdf")).toBe("C:\\");
    expect(parentFolder("/home/a/x.pdf")).toBe("/home/a");
    expect(baseName("C:\\Users\\a\\Documents\\")).toBe("Documents");
    expect(baseName("D:\\")).toBe("D:");
  });

  it("shortens the home folder only at a segment boundary", () => {
    const home = "C:\\Users\\a";
    expect(displayFolder("C:\\Users\\a\\Desktop\\x.pdf", home)).toBe("~\\Desktop");
    expect(displayFolder("C:\\Users\\a\\x.pdf", home)).toBe("~");
    expect(displayFolder("C:\\Users\\ab\\x.pdf", home)).toBe("C:\\Users\\ab");
    expect(displayFolder("D:\\Kuliah\\x.pdf", home)).toBe("D:\\Kuliah");
    expect(displayFolder("D:\\Kuliah\\x.pdf", null)).toBe("D:\\Kuliah");
  });
});

describe("frequentFolders", () => {
  it("orders by count, then by the most recent open", () => {
    const folders = frequentFolders([
      file("C:\\A\\1.pdf", 1),
      file("C:\\B\\1.pdf", 0),
      file("C:\\A\\2.pdf", 5),
      file("C:\\C\\1.pdf", 2),
    ]);
    expect(folders).toEqual(["C:\\A", "C:\\B", "C:\\C"]);
  });

  it("leaves out folders the navigation already names", () => {
    const folders = frequentFolders(
      [file("C:\\U\\Desktop\\a.pdf", 0), file("C:\\U\\Kuliah\\b.pdf", 1)],
      4,
      ["C:\\U\\Desktop\\"],
    );
    expect(folders).toEqual(["C:\\U\\Kuliah"]);
  });

  it("honours the limit", () => {
    const many = Array.from({ length: 9 }, (_, i) => file(`C:\\F${i}\\x.pdf`, i));
    expect(frequentFolders(many, 4)).toHaveLength(4);
  });
});

describe("formatting", () => {
  it("formats sizes with a decimal comma from megabytes up", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(482 * 1024)).toBe("482 KB");
    expect(formatBytes(16.5 * 1024 * 1024)).toBe("16,5 MB");
    expect(formatBytes(null)).toBe("");
  });

  it("formats dates in ISO order", () => {
    expect(formatDate(Date.UTC(2020, 11, 13, 5) / 1000)).toBe("2020-12-13");
    expect(formatDate(null)).toBe("");
    expect(formatDate(0)).toBe("");
  });
});
