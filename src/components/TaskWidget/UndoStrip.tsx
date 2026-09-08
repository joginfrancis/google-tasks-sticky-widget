interface Props {
  title: string;
  /** Subtasks going with it. */
  extra: number;
  onUndo: () => void;
}

/**
 * The only thing standing between a mis-click and permanent data loss, since
 * Google Tasks has no recycle bin.
 *
 * Sits in the layout rather than floating over the list: a toast covering the
 * rows would hide exactly what the user is checking.
 */
export function UndoStrip({ title, extra, onUndo }: Props) {
  return (
    <div className="undo-strip" role="status">
      <span className="undo-text">
        Deleted “{title}”
        {extra > 0 && ` and ${extra} subtask${extra === 1 ? "" : "s"}`}
      </span>
      <button className="undo-button" onClick={onUndo}>
        Undo
      </button>
    </div>
  );
}
