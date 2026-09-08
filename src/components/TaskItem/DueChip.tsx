import { useState } from "react";
import "./DueChip.css";

interface Props {
  label: string;
  overdue: boolean;
  done: boolean;
  open: boolean;
  /** Current due date as an RFC3339 string, or null. */
  value: string | null;
  onOpen: () => void;
  onClose: () => void;
  onChange: (due: string | null) => void;
  /** Used when the picker is opened from the menu and there is no chip yet. */
  hideTrigger?: boolean;
}

const WEEKDAYS = ["S", "M", "T", "W", "T", "F", "S"];

/**
 * Due-date pill plus a month calendar.
 *
 * Google's picker has a "Set time" field. Ours deliberately does not: the API
 * discards the time component (docs/api-findings.md §2), so offering one would
 * appear to work and silently lose the value.
 */
export function DueChip(props: Props) {
  const initial = props.value ? new Date(props.value) : new Date();
  const [month, setMonth] = useState(
    () => new Date(initial.getFullYear(), initial.getMonth(), 1),
  );

  const today = startOfDay(new Date());
  const selected = props.value ? startOfDay(new Date(props.value)) : null;

  const cells = monthCells(month);

  return (
    <span className="due-wrap">
      {!props.hideTrigger && (
        <button
          className={`due-chip ${props.overdue ? "is-overdue" : ""} ${props.done ? "is-done" : ""}`}
          onClick={(e) => {
            e.stopPropagation();
            props.onOpen();
          }}
          title="Change date"
        >
          <svg viewBox="0 0 16 16" width="11" height="11" aria-hidden="true">
            <circle
              cx="8"
              cy="8"
              r="6"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.4"
            />
            <path
              d="M8 4.6V8l2.2 1.6"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.4"
              strokeLinecap="round"
            />
          </svg>
          {props.label}
        </button>
      )}

      {props.open && (
        <>
          <div className="menu-scrim" onClick={props.onClose} />
          <div className="calendar">
            <div className="calendar-head">
              <button
                className="calendar-nav"
                aria-label="Previous month"
                onClick={() => setMonth(addMonths(month, -1))}
              >
                ‹
              </button>
              <span className="calendar-title">
                {month.toLocaleDateString(undefined, {
                  month: "long",
                  year: "numeric",
                })}
              </span>
              <button
                className="calendar-nav"
                aria-label="Next month"
                onClick={() => setMonth(addMonths(month, 1))}
              >
                ›
              </button>
            </div>

            <div className="calendar-grid">
              {WEEKDAYS.map((day, i) => (
                <span key={i} className="calendar-weekday">
                  {day}
                </span>
              ))}

              {cells.map((date, i) => {
                const outside = date.getMonth() !== month.getMonth();
                const isToday = sameDay(date, today);
                const isSelected = selected !== null && sameDay(date, selected);

                return (
                  <button
                    key={i}
                    className={[
                      "calendar-day",
                      outside ? "is-outside" : "",
                      isToday ? "is-today" : "",
                      isSelected ? "is-selected" : "",
                    ]
                      .filter(Boolean)
                      .join(" ")}
                    onClick={() => props.onChange(toApiDate(date))}
                  >
                    {date.getDate()}
                  </button>
                );
              })}
            </div>

            <div className="calendar-actions">
              <button
                className="calendar-action"
                onClick={() => props.onChange(toApiDate(today))}
              >
                Today
              </button>
              <button
                className="calendar-action"
                onClick={() => props.onChange(toApiDate(addDays(today, 1)))}
              >
                Tomorrow
              </button>
              {props.value && (
                <button
                  className="calendar-action is-danger"
                  onClick={() => props.onChange(null)}
                >
                  Clear
                </button>
              )}
            </div>

            <p className="calendar-note">
              Google Tasks doesn't store a time of day.
            </p>
          </div>
        </>
      )}
    </span>
  );
}

/* -- date helpers ---------------------------------------------------------- */

function startOfDay(date: Date): Date {
  const d = new Date(date);
  d.setHours(0, 0, 0, 0);
  return d;
}

function sameDay(a: Date, b: Date): boolean {
  return (
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  );
}

function addDays(date: Date, days: number): Date {
  const d = new Date(date);
  d.setDate(d.getDate() + days);
  return d;
}

function addMonths(date: Date, months: number): Date {
  return new Date(date.getFullYear(), date.getMonth() + months, 1);
}

/**
 * The API wants midnight UTC. Building it from the local Y/M/D rather than
 * converting a local timestamp keeps "the 9th" the 9th — a plain toISOString()
 * would shift the date by one in any timezone behind UTC.
 */
function toApiDate(date: Date): string {
  const y = date.getFullYear();
  const m = String(date.getMonth() + 1).padStart(2, "0");
  const d = String(date.getDate()).padStart(2, "0");
  return `${y}-${m}-${d}T00:00:00.000Z`;
}

/** Six weeks from the Sunday on or before the 1st — a stable 42-cell grid. */
function monthCells(month: Date): Date[] {
  const first = new Date(month.getFullYear(), month.getMonth(), 1);
  const start = addDays(first, -first.getDay());
  return Array.from({ length: 42 }, (_, i) => addDays(start, i));
}
