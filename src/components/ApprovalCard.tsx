/**
 * A Claude Code permission request, answered from the notch.
 *
 * The card says exactly what Allow authorises -- `Bash · cargo publish`, not
 * just "Bash" -- because approving blind is the thing this card exists to
 * avoid. Nothing is approved without a click, and if nobody answers the
 * terminal takes over when the countdown runs out.
 */

import { useEffect, useState } from "react";
import { ShieldAlert } from "lucide-react";

import type { ApprovalRequest } from "../types";

interface Props {
  request: ApprovalRequest;
  onDecide: (decision: "allow" | "deny") => void;
  /** Called once the card is on screen, so the backend starts waiting for a human. */
  onShown: (id: string) => void;
}

function remaining(expiresAt: number, now: number): string {
  const secs = Math.max(0, Math.round((expiresAt - now) / 1000));
  return `${Math.floor(secs / 60)}:${String(secs % 60).padStart(2, "0")}`;
}

export function ApprovalCard({ request, onDecide, onShown }: Props) {
  const [now, setNow] = useState(() => Date.now());
  const [decided, setDecided] = useState(false);

  useEffect(() => {
    onShown(request.id);
    setDecided(false);
  }, [request.id, onShown]);

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);

  const decide = (decision: "allow" | "deny") => {
    if (decided) return;
    setDecided(true);
    onDecide(decision);
  };

  return (
    <div className="approval" role="alertdialog" aria-label="Claude Code permission request">
      <header className="popover-head">
        <ShieldAlert className="popover-mark approval-mark" aria-hidden />
        <span className="popover-title">Needs approval</span>
      </header>
      <p className="popover-account">Claude Code · {request.project}</p>

      <pre className="approval-target thin-scroll">{request.target}</pre>

      <div className="approval-actions">
        <button
          type="button"
          className="approval-btn approval-deny"
          onClick={() => decide("deny")}
          disabled={decided}
        >
          Deny
        </button>
        <button
          type="button"
          className="approval-btn approval-allow"
          onClick={() => decide("allow")}
          disabled={decided}
        >
          Allow
        </button>
      </div>
      <p className="approval-expiry tnum">
        The terminal asks instead in {remaining(request.expiresAt, now)}
      </p>
    </div>
  );
}
