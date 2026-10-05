import { invoke } from "@tauri-apps/api/core";
import { describeLink, parseBlocks, type Inline } from "../../lib/richText";

/**
 * A description, rendered.
 *
 * The text itself is untouched plain text on Google's side; this is only how
 * it is drawn here. Nothing is interpreted as HTML — every token is set as
 * text into an element this file chose — so a description that contains
 * markup is still just a description.
 */
export function RichText({ text }: { text: string }) {
  const blocks = parseBlocks(text);

  return (
    <>
      {blocks.map((block, i) => {
        if (block.kind === "blank") return <span key={i} className="rt-gap" />;
        if (block.kind === "bullet") {
          return (
            <span key={i} className="rt-line is-bullet">
              <span className="rt-marker" aria-hidden="true">
                •
              </span>
              <span>{renderInline(block.content)}</span>
            </span>
          );
        }
        if (block.kind === "numbered") {
          return (
            <span key={i} className="rt-line is-bullet">
              <span className="rt-marker" aria-hidden="true">
                {block.number}.
              </span>
              <span>{renderInline(block.content)}</span>
            </span>
          );
        }
        return (
          <span key={i} className="rt-line">
            {renderInline(block.content)}
          </span>
        );
      })}
    </>
  );
}

function renderInline(tokens: Inline[]) {
  return tokens.map((token, i) => {
    switch (token.kind) {
      case "bold":
        return <strong key={i}>{token.text}</strong>;
      case "italic":
        return <em key={i}>{token.text}</em>;
      case "strike":
        return <s key={i}>{token.text}</s>;
      case "code":
        return (
          <code key={i} className="rt-code">
            {token.text}
          </code>
        );
      case "image":
        return (
          <img
            key={i}
            className="rt-image"
            src={token.url}
            alt={token.alt || "Image"}
            loading="lazy"
            // A picture that will not load should not leave a broken icon in
            // the middle of a note; the link is still in the text.
            onError={(e) => {
              e.currentTarget.classList.add("is-broken");
            }}
            onClick={(e) => {
              e.stopPropagation();
              void invoke("open_link", { url: token.url }).catch(() => {});
            }}
            title="Open in your browser"
          />
        );
      case "link": {
        const open = (e: React.MouseEvent) => {
          // The row would otherwise take this click as "edit me".
          e.stopPropagation();
          void invoke("open_link", { url: token.url }).catch(() => {});
        };

        // A link someone gave words to reads as those words. A bare address
        // becomes a chip saying what it points at — nobody reads an id.
        if (token.text !== token.url) {
          return (
            <button key={i} type="button" className="rt-link" title={token.url} onClick={open}>
              {token.text}
            </button>
          );
        }

        const { host, kind } = describeLink(token.url);
        return (
          <button
            key={i}
            type="button"
            className="rt-card"
            title={token.url}
            onClick={open}
          >
            <span className="rt-card-mark" aria-hidden="true">
              {host.slice(0, 1).toUpperCase()}
            </span>
            <span className="rt-card-text">
              <span className="rt-card-host">{host}</span>
              <span className="rt-card-kind">{kind}</span>
            </span>
          </button>
        );
      }
      default:
        return <span key={i}>{token.text}</span>;
    }
  });
}
