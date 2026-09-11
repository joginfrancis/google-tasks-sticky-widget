import { useRef, useState } from "react";

interface Props {
  value: string | null;
  onChange: (color: string | null) => void;
}

/**
 * Sticky-note colours: a wheel on the right that opens the palette to its left.
 *
 * Collapsed by default so the bar costs one icon rather than a full strip —
 * colour is set rarely and looked at never.
 *
 * The palette is deliberately muted: these tint a panel that sits on screen all
 * day with text on it, so they are pale washes rather than saturated colours.
 */
const PRESETS: { name: string; value: string }[] = [
  { name: "Yellow", value: "#fdf3c6" },
  { name: "Green", value: "#d8f0d3" },
  { name: "Blue", value: "#d6e8f7" },
  { name: "Pink", value: "#f9dde6" },
  { name: "Purple", value: "#e5ddf5" },
  { name: "Orange", value: "#fbe2cd" },
  { name: "Grey", value: "#e4e5e8" },
];

interface Swatch {
  key: string;
  className: string;
  style?: React.CSSProperties;
  title: string;
  active: boolean;
  content?: React.ReactNode;
  onClick: () => void;
}

/** Built as data so the roll animation can index over one uniform list. */
function SWATCHES(
  value: string | null,
  onChange: (color: string | null) => void,
  openCustom: () => void,
): Swatch[] {
  return [
    {
      key: "default",
      // The absence of a colour, drawn as a diagonal rather than as another
      // colour that could be mistaken for a choice.
      className: `swatch is-default ${value === null ? "is-active" : ""}`,
      title: "Default",
      active: value === null,
      onClick: () => onChange(null),
    },
    ...PRESETS.map((preset) => ({
      key: preset.name,
      className: `swatch ${value === preset.value ? "is-active" : ""}`,
      style: { background: preset.value },
      title: preset.name,
      active: value === preset.value,
      onClick: () => onChange(preset.value),
    })),
    {
      key: "custom",
      // "A colour not on this row" — opens the OS picker rather than being a
      // colour itself.
      // A colour wheel rather than a "+": the row it sits in is already a set of
      // colours, so "another one of these" needs no explaining, and the wheel
      // says *any* colour where a plus only said "more". It can afford to be
      // vivid here — it only appears once the palette is open, unlike the
      // trigger, which sits on the note all the time and must stay quiet.
      className: "swatch is-add",
      title: "Custom colour…",
      active: false,
      onClick: openCustom,
    },
  ];
}

export function ColorBar({ value, onChange }: Props) {
  const [open, setOpen] = useState(false);
  const customRef = useRef<HTMLInputElement>(null);

  return (
    <div className="color-bar">
      {open && (
        <div className="color-palette">
          {/*
            Each swatch rolls out from under the wheel to its place, like a
            pebble coming to rest: the ones travelling furthest leave first, so
            they arrive together rather than in a queue.

            `--roll` is the distance in swatch-widths, which drives both the
            travel and the spin — a ball that translates without rotating reads
            as sliding, not rolling.
          */}
          {SWATCHES(value, onChange, () => customRef.current?.click()).map(
            (swatch, index, all) => (
              <button
                key={swatch.key}
                className={swatch.className}
                style={
                  {
                    ...swatch.style,
                    "--roll": all.length - index,
                    animationDelay: `${index * 26}ms`,
                  } as React.CSSProperties
                }
                title={swatch.title}
                aria-label={swatch.title}
                aria-pressed={swatch.active}
                onClick={swatch.onClick}
              >
                {swatch.content}
              </button>
            ),
          )}
        </div>
      )}

      {/*
        Deliberately quiet: the note's own colour with a thin ring in the
        derived text colour. Because that colour is computed from the
        background, the ring always reads against whatever the note is — dark
        on pale notes, light on dark ones — without ever competing with the
        tasks for attention.

        The rainbow wheel it replaced announced itself far too loudly for a
        control used once and then ignored.
      */}
      <button
        className={`color-wheel ${open ? "is-open" : ""}`}
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        aria-label={open ? "Hide colours" : "Note colour"}
        title="Note colour"
      />

      {/* Native picker, driven by the blank swatch. Windows' own is better than
          anything worth hand-rolling, and it is one line. */}
      <input
        ref={customRef}
        className="color-input"
        type="color"
        value={value ?? "#fdf3c6"}
        onChange={(e) => onChange(e.target.value)}
        tabIndex={-1}
        aria-hidden="true"
      />
    </div>
  );
}

/**
 * Text colour derived from the background rather than chosen separately.
 *
 * If both were pickable, someone eventually lands on an unreadable pair — and a
 * sticky note you cannot read is worse than a plain white one. Uses relative
 * luminance so it holds for any custom colour, not just the presets.
 */
export function readableText(background: string): string {
  const hex = background.replace("#", "");
  if (hex.length !== 6) return "#1a1a1f";

  const channel = (offset: number) => {
    const v = parseInt(hex.slice(offset, offset + 2), 16) / 255;
    return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
  };

  const luminance =
    0.2126 * channel(0) + 0.7152 * channel(2) + 0.0722 * channel(4);

  // 0.45 rather than 0.5: dark text on a mid tone stays comfortable further
  // down the range than light text does going up.
  return luminance > 0.45 ? "#1a1a1f" : "#f5f5f7";
}
