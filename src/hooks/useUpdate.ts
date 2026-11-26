import { useCallback, useEffect, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { getAllWindows } from "@tauri-apps/api/window";
import { pickChecker, windowLabel } from "../lib/window";
import { uiLog } from "../lib/log";
import { FIRST_CHECK_MS, nextCheckMs, type CheckOutcome } from "../lib/updateSchedule";

/**
 * Checking for, and installing, a new version.
 *
 * Deliberately quiet: a new version is offered on a strip the user can ignore.
 * A widget that interrupts you to talk about itself is a widget you close.
 *
 * Quiet is not the same as mute, which is what 0.1.6 was. It looked once,
 * twenty seconds after launch, discarded any failure without a word, and did
 * not look again until the next launch. 0.1.8 duly sat on GitHub for nine days
 * without being offered on the desk it was built for, and the evidence for why
 * amounts to a single line - one check that got a non-success status from the
 * endpoint, on 8 November - because a success and a client-side failure both
 * logged nothing at all. Twenty seconds is also a poor moment to ask: a laptop
 * opened at a desk is awake before it is online.
 *
 * So now: retried with a backing-off delay, asked again when the network
 * returns, asked again hours later even on success, and every outcome written
 * to the log. Exactly one note does the asking - see `pickChecker` - and the
 * election is re-run before each attempt so the job passes on when that note
 * is closed.
 */

export type UpdateState =
  | { phase: "idle" }
  | { phase: "available"; version: string }
  | { phase: "downloading" }
  | { phase: "ready" }
  | { phase: "failed"; message: string };

export function useUpdate() {
  const [state, setState] = useState<UpdateState>({ phase: "idle" });
  const [update, setUpdate] = useState<Update | null>(null);

  useEffect(() => {
    let cancelled = false;
    let timer = 0;
    let failures = 0;
    let running = false;

    const schedule = (outcome: CheckOutcome) => {
      if (cancelled) return;
      window.clearTimeout(timer);
      timer = window.setTimeout(attempt, nextCheckMs(outcome, failures));
    };

    async function attempt() {
      if (cancelled || running) return;
      running = true;
      try {
        // Asked for now rather than at mount: the arrangement is still being
        // rebuilt when the first note's effect runs, so a list taken then
        // would be missing the notes that open after it.
        const labels = (await getAllWindows()).map((w) => w.label);
        if (cancelled) return;
        if (pickChecker(labels) !== windowLabel()) {
          schedule("standby");
          return;
        }

        const found = await check();
        if (cancelled) return;
        failures = 0;
        if (!found) {
          schedule("current");
          return;
        }
        // Nothing more to ask: the strip is the user's business now.
        uiLog(`update: ${found.version} offered`);
        setUpdate(found);
        setState({ phase: "available", version: found.version });
      } catch (err) {
        if (cancelled) return;
        failures += 1;
        // The one thing 0.1.6 got right, and only in Rust. A check nobody can
        // see the result of is a check nobody can diagnose.
        uiLog(`update: check failed (${failures} in a row): ${String(err)}`, "warn");
        schedule("failed");
      } finally {
        running = false;
      }
    }

    // Awake before online: the gap that swallowed the single attempt. Only
    // worth acting on after a failure - otherwise every flicker of the
    // connection would start a check we already have the answer to.
    const onOnline = () => {
      if (cancelled || running || failures === 0) return;
      uiLog("update: the network is back; looking again");
      failures = 0;
      window.clearTimeout(timer);
      timer = window.setTimeout(attempt, 0);
    };
    window.addEventListener("online", onOnline);

    timer = window.setTimeout(attempt, FIRST_CHECK_MS);

    return () => {
      cancelled = true;
      window.clearTimeout(timer);
      window.removeEventListener("online", onOnline);
    };
  }, []);

  const install = useCallback(async () => {
    if (!update) return;
    setState({ phase: "downloading" });
    try {
      await update.downloadAndInstall();
      setState({ phase: "ready" });
      // The installer has run; the new version is what starts back up.
      await relaunch();
    } catch (err) {
      uiLog(`update: install failed: ${String(err)}`, "error");
      setState({ phase: "failed", message: String(err) });
    }
  }, [update]);

  const dismiss = useCallback(() => setState({ phase: "idle" }), []);

  return { state, install, dismiss };
}
