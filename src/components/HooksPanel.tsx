/**
 * Installing CodeNotch's Claude Code hooks, from the settings card.
 *
 * Writing `~/.claude/settings.json` is the one time CodeNotch touches another
 * tool's files, so it never happens in one click: the first click only shows
 * the exact diff and where the backup will go, and the file is written only
 * from the button under that diff. If the file changes in between, the write
 * is refused and a fresh diff is needed.
 */

import { useCallback, useEffect, useState } from "react";

import { IN_TAURI, ipc } from "../lib/ipc";
import type { HookPreview, HookStatus } from "../types";

type Phase =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "preview"; preview: HookPreview }
  | { kind: "writing"; preview: HookPreview }
  | { kind: "done"; message: string }
  | { kind: "error"; message: string };

export function HooksPanel() {
  const [status, setStatus] = useState<HookStatus | null>(null);
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });

  const refresh = useCallback(() => {
    void ipc.hooksStatus().then((s) => s && setStatus(s));
  }, []);

  useEffect(refresh, [refresh]);

  if (!IN_TAURI) {
    return (
      <p className="hooks-note">
        Live sessions and approvals from the notch need the desktop app.
      </p>
    );
  }

  const installed = status?.installed ?? false;

  const preview = async (install: boolean) => {
    setPhase({ kind: "loading" });
    try {
      setPhase({ kind: "preview", preview: await ipc.hooksPreview(install) });
    } catch (error) {
      setPhase({ kind: "error", message: String(error).replace(/^Error:\s*/, "") });
    }
  };

  const write = async (plan: HookPreview) => {
    setPhase({ kind: "writing", preview: plan });
    try {
      const backup = await ipc.hooksApply(plan.install, plan.fingerprint);
      setPhase({
        kind: "done",
        message: `${plan.install ? "Installed" : "Removed"}.${backup ? ` Backup: ${backup}` : ""}`,
      });
      refresh();
    } catch (error) {
      setPhase({ kind: "error", message: String(error).replace(/^Error:\s*/, "") });
    }
  };

  if (phase.kind === "preview" || phase.kind === "writing") {
    const plan = phase.preview;
    return (
      <div className="hooks">
        <p className="hooks-note">
          {plan.install ? "Adds" : "Removes"} CodeNotch's entries in{" "}
          <code>{plan.settingsPath}</code>. Nothing else changes. A backup goes to{" "}
          <code>{plan.backup}</code>.
        </p>
        <pre className="hooks-diff thin-scroll">
          {plan.diff.split("\n").map((line, i) => (
            <span key={i} data-sign={line[0]}>
              {line}
              {"\n"}
            </span>
          ))}
        </pre>
        <div className="hooks-actions">
          <button type="button" className="hooks-btn" onClick={() => setPhase({ kind: "idle" })}>
            Cancel
          </button>
          <button
            type="button"
            className="hooks-btn hooks-btn-primary"
            disabled={phase.kind === "writing"}
            onClick={() => void write(plan)}
          >
            Write settings.json
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="hooks">
      <p className="hooks-note">
        {installed
          ? "Installed: live steps and Allow / Deny from the notch."
          : "Optional: live steps and Allow / Deny from the notch. You review the change first."}
        {status && !status.hookReady && " The relay is missing from this build."}
      </p>
      {phase.kind === "done" && <p className="hooks-note hooks-ok">{phase.message}</p>}
      {phase.kind === "error" && <p className="hooks-note hooks-bad">{phase.message}</p>}
      <div className="hooks-actions">
        <button
          type="button"
          className="hooks-btn"
          disabled={phase.kind === "loading" || (!installed && status?.hookReady === false)}
          onClick={() => void preview(!installed)}
        >
          {installed ? "Remove hooks…" : "Install hooks…"}
        </button>
      </div>
    </div>
  );
}
