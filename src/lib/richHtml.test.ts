import { describe, it, expect } from "vitest";
import { toHtml, toMarkdown } from "./richHtml";

/** What the editor would hand back after being opened on `markdown`. */
function roundTrip(markdown: string): string {
  const host = document.createElement("div");
  host.innerHTML = toHtml(markdown);
  return toMarkdown(host);
}

describe("markdown to editable HTML and back", () => {
  it("keeps formatting, links, pictures and bullets", () => {
    const source = [
      "**bold** and *italic* and ~~gone~~",
      "- one",
      "- two",
      "[docs](https://x.test/a)",
      "![](https://x.test/cat.png)",
    ].join("\n");

    expect(roundTrip(source)).toBe(source);
  });

  it("keeps a blank line between paragraphs", () => {
    expect(roundTrip("one\n\ntwo")).toBe("one\n\ntwo");
  });

  it("escapes markup rather than rendering it", () => {
    // A description is user text that has travelled through an API.
    expect(toHtml("<script>alert(1)</script>")).not.toContain("<script>");
    expect(roundTrip("a <b>tag</b> typed by hand")).toBe("a <b>tag</b> typed by hand");
  });

  it("reads what a browser's own editing leaves behind", () => {
    const host = document.createElement("div");
    // execCommand produces any of these, depending on the browser and the day.
    host.innerHTML =
      '<div><b>bold</b> <span style="font-style: italic">slanted</span></div>' +
      '<ul><li>item</li></ul>' +
      '<div><a href="https://x.test">https://x.test</a></div>';

    expect(toMarkdown(host)).toBe("**bold** *slanted*\n- item\nhttps://x.test");
  });

  it("drops markers around nothing", () => {
    const host = document.createElement("div");
    host.innerHTML = "<div><b></b>text</div>";
    expect(toMarkdown(host)).toBe("text");
  });

  it("writes an image the editor inserted, whatever its address looks like", () => {
    const host = document.createElement("div");
    // No file extension: an address like this is still a picture if the user
    // added it as one, which is the difference the read-only view cannot see.
    host.innerHTML = '<div><img src="https://x.test/photo?id=9"></div>';
    expect(toMarkdown(host)).toBe("![](https://x.test/photo?id=9)");
  });
});
