import { useState } from "react";
import type { SyncStatus as Status } from "../../types";
import { formatLastSynced } from "../../lib/date";

interface Props {
  status: Status;
  onSyncNow: () => void;
}

const LABELS: Record<Status["state"], string> = {
  synced: "Up to date",
  syncing: "Syncing…",
  offline: "Offline",
  error: "Sync problem",
};

/**
 * Refresh button and sync indicator in one control.
 *
 * They were two separate things — a status dot and a refresh arrow — which cost
 * two slots in a 340px header and split one idea across them. The state now
 * lives as a coloured dot at the centre of the arrow: green up to date, amber
 * offline, red error, and the arrow spins while a sync is in flight.
 *
 * Click still means "sync now"; click again for detail.
 */
export function SyncButton({ status, onSyncNow }: Props) {
  const [open, setOpen] = useState(false);
  const syncing = status.state === "syncing";

  return (
    <div className="sync-button-wrap">
      <button
        className={`icon-button sync-button is-${status.state}`}
        onClick={onSyncNow}
        onContextMenu={(e) => {
          e.preventDefault();
          setOpen((v) => !v);
        }}
        disabled={syncing}
        aria-label={`${LABELS[status.state]}. Sync now`}
        title={`${LABELS[status.state]} — click to sync (Ctrl+R)`}
      >
        <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
          <g className="sync-arrow">
            <path
              d="M13.4 8a5.4 5.4 0 1 1-1.6-3.8"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.4"
              strokeLinecap="round"
            />
            <path
              d="M13.4 2.2v3.1h-3.1"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.4"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </g>
          {/* The state lives in the middle of the ring, so one glance covers
              both "is it fresh" and "how do I refresh it". */}
          <circle className="sync-core" cx="8" cy="8" r="2.6" />
        </svg>
      </button>

      {open && (
        <>
          <div className="menu-scrim" onClick={() => setOpen(false)} />
          <div className="sync-popover">
            <strong>{LABELS[status.state]}</strong>
            <p className="sync-detail">
              {status.message ?? formatLastSynced(status.lastSyncedAt)}
            </p>
          </div>
        </>
      )}
    </div>
  );
}
