import "./Onboarding.css";

interface Props {
  onConnect: () => void;
  /** A browser tab is open and we are waiting on it. */
  authorizing: boolean;
  error: string | null;
}

/**
 * First run. SPEC §3.1 — the widget must never open to an empty list, and the
 * scope reassurance is part of the design, not marketing: an unverified-app
 * warning is about to appear and the user deserves to know what they granted.
 */
export function Onboarding({ onConnect, authorizing, error }: Props) {
  return (
    <div className="onboarding">
      <div className="onboarding-mark" aria-hidden="true">
        <svg viewBox="0 0 32 32" width="30" height="30">
          <rect
            x="4"
            y="5"
            width="24"
            height="22"
            rx="4"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.6"
          />
          <path
            d="M10 16l4 4 8-9"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      </div>

      <h1>My Google Tasks</h1>
      <p className="onboarding-lead">
        Keep your Google Tasks visible on your desktop.
      </p>

      <button
        className="connect-button"
        onClick={onConnect}
        disabled={authorizing}
      >
        {authorizing ? "Waiting for your browser…" : "Connect Google"}
      </button>

      {authorizing && (
        <p className="onboarding-waiting">
          Finish signing in on the tab that just opened.
        </p>
      )}

      {error && <p className="onboarding-error">{error}</p>}

      <p className="onboarding-privacy">
        Only Google Tasks access is requested — not Gmail, Drive, Contacts, or
        your password. Sign-in happens in your browser.
      </p>
    </div>
  );
}
