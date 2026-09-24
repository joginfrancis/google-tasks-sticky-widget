import { describe, it, expect } from "vitest";
import {
  insertLink,
  toggleBullet,
  parseBlocks,
  parseInline,
  isImageUrl,
  isUrl,
  pasteLink,
  toggleMarker,
} from "./richText";

describe("parseInline", () => {
  it("reads bold, italic and code", () => {
    expect(parseInline("a **b** c *d* `e`")).toEqual([
      { kind: "text", text: "a " },
      { kind: "bold", text: "b" },
      { kind: "text", text: " c " },
      { kind: "italic", text: "d" },
      { kind: "text", text: " " },
      { kind: "code", text: "e" },
    ]);
  });

  it("keeps punctuation inside a URL out of the formatting", () => {
    // Underscores and asterisks are ordinary characters in an address.
    const tokens = parseInline("see https://x.test/a_b_c now");
    expect(tokens[1]).toEqual({
      kind: "link",
      text: "https://x.test/a_b_c",
      url: "https://x.test/a_b_c",
    });
  });

  it("leaves a trailing full stop out of the link", () => {
    const tokens = parseInline("go to https://x.test/page.");
    expect(tokens[1]).toMatchObject({ url: "https://x.test/page" });
    expect(tokens[2]).toEqual({ kind: "text", text: "." });
  });

  it("does not italicise snake_case", () => {
    expect(parseInline("task_list_id")).toEqual([
      { kind: "text", text: "task_list_id" },
    ]);
  });

  it("shows a named link and a picture", () => {
    expect(parseInline("[docs](https://x.test/a)")).toEqual([
      { kind: "link", text: "docs", url: "https://x.test/a" },
    ]);
    expect(parseInline("https://x.test/cat.png")).toEqual([
      { kind: "image", url: "https://x.test/cat.png", alt: "" },
    ]);
  });
});

describe("parseBlocks", () => {
  it("keeps lines, bullets and numbering", () => {
    const blocks = parseBlocks("one\n- two\n3. three\n\nfour");
    expect(blocks.map((b) => b.kind)).toEqual([
      "paragraph",
      "bullet",
      "numbered",
      "blank",
      "paragraph",
    ]);
  });
});

describe("editing helpers", () => {
  it("names a link with the words it was pasted over", () => {
    expect(pasteLink("see docs here", 4, 8, "https://x.test")).toEqual({
      text: "see [docs](https://x.test) here",
      caret: 26,
    });
  });

  it("inserts a bare URL when nothing is selected", () => {
    expect(pasteLink("see ", 4, 4, "https://x.test").text).toBe("see https://x.test");
  });

  it("toggles a marker on and back off", () => {
    const on = toggleMarker("make bold now", 5, 9, "**");
    expect(on.text).toBe("make **bold** now");
    // The selection still covers the word, so pressing again undoes it.
    const off = toggleMarker(on.text, on.start, on.end, "**");
    expect(off.text).toBe("make bold now");
  });

  it("knows a picture and a bare link apart", () => {
    expect(isImageUrl("https://x.test/a.JPG?w=2")).toBe(true);
    expect(isImageUrl("https://x.test/a")).toBe(false);
    expect(isUrl(" https://x.test ")).toBe(true);
    expect(isUrl("https://x.test and more")).toBe(false);
  });
});

describe("toolbar helpers", () => {
  it("bullets every line the selection touches, and unbullets them again", () => {
    const on = toggleBullet("one\ntwo\nthree", 0, 7);
    expect(on.text).toBe("- one\n- two\nthree");

    const off = toggleBullet(on.text, on.start, on.end);
    expect(off.text).toBe("one\ntwo\nthree");
  });

  it("leaves blank lines alone", () => {
    expect(toggleBullet("one\n\ntwo", 0, 8).text).toBe("- one\n\n- two");
  });

  it("names a link with the selection, and writes an image differently", () => {
    expect(insertLink("see docs", 4, 8, "https://x.test").text).toBe(
      "see [docs](https://x.test)",
    );
    expect(insertLink("", 0, 0, "https://x.test/a.png", true).text).toBe(
      "![](https://x.test/a.png)",
    );
  });

  it("reads strikethrough", () => {
    expect(parseInline("~~gone~~")).toEqual([{ kind: "strike", text: "gone" }]);
  });
});
