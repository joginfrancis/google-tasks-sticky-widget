/**
 * Due-date formatting.
 *
 * Google Tasks due dates carry no time (SPEC §4.1), so every comparison here is
 * calendar-day based. Using timestamps would make "today" flip at UTC midnight
 * rather than the user's, which reads as a bug in the evening.
 */

function startOfLocalDay(date: Date): Date {
  const d = new Date(date);
  d.setHours(0, 0, 0, 0);
  return d;
}

/** Whole days from today. Negative is in the past. */
export function daysFromToday(due: string): number {
  const dueDay = startOfLocalDay(new Date(due));
  const today = startOfLocalDay(new Date());
  const msPerDay = 24 * 60 * 60 * 1000;
  return Math.round((dueDay.getTime() - today.getTime()) / msPerDay);
}

export function isOverdue(due: string | null): boolean {
  if (!due) return false;
  return daysFromToday(due) < 0;
}

export function isDueToday(due: string | null): boolean {
  if (!due) return false;
  return daysFromToday(due) === 0;
}

/**
 * Short enough for a badge on a 340px panel: "Today", "Tue", "12 Sep".
 * Never a time — there isn't one to show.
 */
export function formatDue(due: string): string {
  const diff = daysFromToday(due);
  const date = new Date(due);

  if (diff === 0) return "Today";
  if (diff === 1) return "Tomorrow";
  if (diff === -1) return "Yesterday";

  // Inside the coming week a weekday name is the most readable form.
  if (diff > 1 && diff < 7) {
    return date.toLocaleDateString(undefined, { weekday: "short" });
  }

  if (diff < -1 && diff > -7) {
    return `${Math.abs(diff)} days ago`;
  }

  const sameYear = date.getFullYear() === new Date().getFullYear();
  return date.toLocaleDateString(undefined, {
    day: "numeric",
    month: "short",
    ...(sameYear ? {} : { year: "numeric" }),
  });
}

export function formatLastSynced(iso: string | null): string {
  if (!iso) return "Never synced";

  const seconds = Math.round((Date.now() - new Date(iso).getTime()) / 1000);
  if (seconds < 10) return "Synced just now";
  if (seconds < 60) return `Synced ${seconds}s ago`;

  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `Synced ${minutes} min ago`;

  const hours = Math.round(minutes / 60);
  if (hours < 24) return `Synced ${hours}h ago`;

  return `Synced ${new Date(iso).toLocaleDateString()}`;
}
