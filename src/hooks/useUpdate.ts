import { useCallback, useEffect, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { isMainNote } from "../lib/window";

/**
 * Checking for, and installing, a new version.
 *
 * Deliberately quiet: the check happens once, a little after launch, and a
 * new version is offered on a strip the user can ignore. A widget that
 * interrupts you to talk about itself is a widget you close.
 *
 * Only the main note checks. Four notes are four webviews of the same app, and
 * four simultaneous downloads of the same installer would be absurd.
 */
const CHECK_DELAY_MS = 20_000;

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
    if (!isMainNote()) return;

    let cancelled = false;
    const timer = window.setTimeout(() => {
      check()
        .then((found) => {
          if (cancelled || !found) return;
          setUpdate(found);
          setState({ phase: "available", version: found.version });
        })
        // An update check failing is not the user's problem: no network, no
        // release yet, a GitHub hiccup. Stay silent and try again next launch.
        .catch(() => {});
    }, CHECK_DELAY_MS);

    return () => {
      cancelled = true;
      window.clearTimeout(timer);
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
      setState({ phase: "failed", message: String(err) });
    }
  }, [update]);

  const dismiss = useCallback(() => setState({ phase: "idle" }), []);

  return { state, install, dismiss };
}
