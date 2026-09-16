interface Props {
  title: string;
  /** Subtasks going with it. */
  extra: number;
  /** How many tasks went together; above one, the count is what is shown. */
  count?: number;
  onUndo: () => void;
}

/**
 * The only thing standing between a mis-click and permanent data loss, since
 * Google Tasks has no recycle bin.
 *
 * Sits in the layout rather than floating over the list: a toast covering the
 * rows would hide exactly what the user is checking.
 */
export function UndoStrip({ title, extra, count = 1, onUndo }: Props) {
  return (
    <div className="undo-strip" role="status">
      <span className="undo-text">
        {count > 1 ? `Deleted ${count} tasks` : `Deleted “${title}”`}
        {extra > 0 && ` and ${extra} subtask${extra === 1 ? "" : "s"}`}
      </span>
      <button className="undo-button" onClick={onUndo}>
        Undo
      </button>
    </div>
  );
}
