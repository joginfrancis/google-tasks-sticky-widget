import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { uiLog } from "../../lib/log";

/**
 * The section you need on the day the widget stops answering.
 *
 * Every freeze so far has been argued about from a log that simply stops: the
 * last line is from an ordinary moment and nothing follows it, because the
 * thread that writes the log is the thread that froze. A watchdog now runs off
 * that thread and records the stall as it happens, and the app notices on the
 * next launch that the previous run never said goodbye. This panel is where
 * that evidence is picked up.
 *
 * Three doors, deliberately plain:
 *
 * - the folder, because "where are the logs" should not need an answer in
 *   prose that gets retyped wrongly;
 * - a report saved as a file, because that is the thing you can attach;
 * - the same report on screen, because sending a file you have not read is
 *   not consent.
 */

type Status =
  | { kind: "idle" }
  | { kind: "busy" }
  | { kind: "saved"; path: string }
  | { kind: "copied" }
  | { kind: "failed"; message: string };

export function Troubleshooting() {
  const [lastSession, setLastSession] = useState<string>("");
  const [status, setStatus] = useState<Status>({ kind: "idle" });
  const [report, setReport] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    invoke<string>("last_session_summary")
      .then((summary) => {
        if (!cancelled) setLastSession(summary);
      })
      .catch(() => {
        // A missing verdict is not worth a row of its own.
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const openFolder = async () => {
    try {
      await invoke("open_log_folder");
    } catch (err) {
      setStatus({ kind: "failed", message: String(err) });
    }
  };

  const save = async () => {
    setStatus({ kind: "busy" });
    try {
      const path = await invoke<string>("save_diagnostic_report");
      uiLog("diagnostics: report saved from the panel");
      setStatus({ kind: "saved", path });
    } catch (err) {
      setStatus({ kind: "failed", message: String(err) });
    }
  };

  const show = async () => {
    setStatus({ kind: "busy" });
    try {
      const text = await invoke<string>("diagnostic_report");
      setReport(text);
      setStatus({ kind: "idle" });
    } catch (err) {
      setStatus({ kind: "failed", message: String(err) });
    }
  };

  const copy = async () => {
    try {
      const text = report ?? (await invoke<string>("diagnostic_report"));
      await navigator.clipboard.writeText(text);
      setStatus({ kind: "copied" });
    } catch (err) {
      // Clipboard access can be refused; saving the file always works.
      setStatus({
        kind: "failed",
        message: `Could not copy (${String(err)}). Save the report instead.`,
      });
    }
  };

  return (
    <section>
      <h2>Troubleshooting</h2>

      {lastSession && <p className="settings-hint">{lastSession}</p>}

      <div className="settings-actions">
        <button className="settings-button" onClick={openFolder}>
          Open log folder
        </button>
        <button
          className="settings-button"
          onClick={save}
          disabled={status.kind === "busy"}
        >
          Save report
        </button>
      </div>
      <div className="settings-actions">
        <button
          className="settings-button"
          onClick={report ? () => setReport(null) : show}
          disabled={status.kind === "busy"}
        >
          {report ? "Hide report" : "Show report"}
        </button>
        <button
          className="settings-button"
          onClick={copy}
          disabled={status.kind === "busy"}
        >
          Copy report
        </button>
      </div>

      {status.kind === "saved" && (
        <p className="settings-hint">Saved to {status.path}</p>
      )}
      {status.kind === "copied" && (
        <p className="settings-hint">Report copied to the clipboard.</p>
      )}
      {status.kind === "failed" && (
        <p className="settings-error">{status.message}</p>
      )}

      {report && <pre className="diagnostic-report">{report}</pre>}

      <p className="settings-note">
        The report holds versions, screen sizes, window positions and recent log
        lines. It does not contain your tasks, their contents, or your account
        details — read it before sending it anywhere.
      </p>
    </section>
  );
}
