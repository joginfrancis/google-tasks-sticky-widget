import type { WindowLayer } from "../../types";

interface Props {
  layer: WindowLayer;
  onChange: (layer: WindowLayer) => void;
}

/**
 * Pinned on top, or an ordinary window. Two states, not three.
 *
 * "Behind other windows" was dropped: on a note you summon with a hotkey and
 * dismiss to the tray, a window that deliberately hides under everything has no
 * job to do, and a third state made a one-click control need thinking about.
 * The backend still understands `bottom` for older settings files.
 */
export function PinControl({ layer, onChange }: Props) {
  const pinned = layer === "top";

  return (
    <button
      className={`icon-button pin-button ${pinned ? "is-pinned" : ""}`}
      onClick={() => onChange(pinned ? "normal" : "top")}
      aria-pressed={pinned}
      aria-label={pinned ? "Pinned on top" : "Not pinned"}
      title={pinned ? "Pinned on top — click to unpin" : "Not pinned — click to pin on top"}
    >
      <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
        {/* A drawing-pin seen head-on: round head, tapering shaft. Reads as a
            pin at 14px, which the angled push-pin outline did not. */}
        <path
          d="M8 1.6a3.4 3.4 0 0 0-3.4 3.4c0 .9.5 1.7 1.1 2.3.5.5.8 1.1.9 1.8l.2 1.1h2.4l.2-1.1c.1-.7.4-1.3.9-1.8.6-.6 1.1-1.4 1.1-2.3A3.4 3.4 0 0 0 8 1.6Z"
          fill={pinned ? "currentColor" : "none"}
          stroke="currentColor"
          strokeWidth="1.3"
          strokeLinejoin="round"
        />
        <path
          d="M8 10.2V14.4"
          stroke="currentColor"
          strokeWidth="1.3"
          strokeLinecap="round"
        />
      </svg>
    </button>
  );
}
