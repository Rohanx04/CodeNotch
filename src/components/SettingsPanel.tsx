/** The settings view inside the expanded card. */

import { ArrowLeft, FolderOpen, Power } from "lucide-react";

import type { Config, Edge, HudSize, MonitorInfo, ProviderId } from "../types";

const EDGES: { value: Edge; label: string }[] = [
  { value: "top", label: "Top" },
  { value: "bottom", label: "Bottom" },
  { value: "left", label: "Left" },
  { value: "right", label: "Right" },
];

const SIZES: { value: HudSize; label: string }[] = [
  { value: "small", label: "S" },
  { value: "medium", label: "M" },
  { value: "large", label: "L" },
];

const ACCENTS = ["#22d3ee", "#a78bfa", "#34d399", "#fbbf24", "#fb7185", "#60a5fa"];

const PROVIDERS: { id: ProviderId; label: string }[] = [
  { id: "claudeCode", label: "Claude Code" },
  { id: "cursor", label: "Cursor" },
  { id: "copilot", label: "GitHub Copilot" },
  { id: "codex", label: "Codex" },
  { id: "ollama", label: "Ollama" },
];

interface Props {
  config: Config;
  monitors: MonitorInfo[];
  version: string;
  onChange: (config: Config) => void;
  onClose: () => void;
  onOpenConfigDir: () => void;
  onQuit: () => void;
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-2 py-[5px]">
      <span className="text-[11px] text-notch-muted">{label}</span>
      {children}
    </div>
  );
}

function Toggle({
  checked,
  onChange,
  label,
}: {
  checked: boolean;
  onChange: (value: boolean) => void;
  label: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      onClick={() => onChange(!checked)}
      className={`relative h-[16px] w-[30px] shrink-0 cursor-pointer rounded-full transition-colors ${
        checked ? "bg-[var(--accent)]" : "bg-white/15"
      }`}
    >
      <span
        className="absolute top-[2px] size-[12px] rounded-full bg-white transition-[left] duration-150"
        style={{ left: checked ? 16 : 2 }}
      />
    </button>
  );
}

function SegmentedControl<T extends string>({
  value,
  options,
  onChange,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
}) {
  return (
    <div className="flex shrink-0 gap-[2px] rounded-md bg-white/6 p-[2px]">
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          onClick={() => onChange(option.value)}
          className={`cursor-pointer rounded px-1.5 py-[2px] text-[10px] transition-colors ${
            value === option.value
              ? "bg-white/14 text-notch-text"
              : "text-notch-faint hover:text-notch-muted"
          }`}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

export function SettingsPanel({
  config,
  monitors,
  version,
  onChange,
  onClose,
  onOpenConfigDir,
  onQuit,
}: Props) {
  const patch = (changes: Partial<Config>) => onChange({ ...config, ...changes });

  return (
    <div className="flex flex-col">
      <div className="flex items-center gap-1.5 border-b border-white/8 px-2.5 py-2">
        <button
          type="button"
          onClick={onClose}
          aria-label="Back"
          className="cursor-pointer rounded p-1 text-notch-muted transition-colors hover:bg-white/8 hover:text-notch-text"
        >
          <ArrowLeft className="size-3" />
        </button>
        <span className="flex-1 text-[12px] font-medium">Settings</span>
        <span className="tnum text-[9px] text-notch-faint">v{version}</span>
      </div>

      <div className="thin-scroll max-h-[420px] overflow-y-auto px-2.5 py-1">
        <Row label="Screen edge">
          <SegmentedControl
            value={config.edge}
            options={EDGES}
            onChange={(edge) => patch({ edge })}
          />
        </Row>

        <Row label="Size">
          <SegmentedControl
            value={config.size}
            options={SIZES}
            onChange={(size) => patch({ size })}
          />
        </Row>

        <Row label="Position along edge">
          <input
            type="range"
            min={0}
            max={1}
            step={0.05}
            value={config.edgeOffset}
            onChange={(e) => patch({ edgeOffset: Number(e.target.value) })}
            className="h-1 w-[110px] shrink-0 cursor-pointer accent-[var(--accent)]"
            aria-label="Position along edge"
          />
        </Row>

        <Row label="Accent">
          <div className="flex shrink-0 gap-1">
            {ACCENTS.map((colour) => (
              <button
                key={colour}
                type="button"
                aria-label={`Accent ${colour}`}
                onClick={() => patch({ accent: colour })}
                className={`size-[14px] cursor-pointer rounded-full transition-transform ${
                  config.accent === colour
                    ? "ring-2 ring-white/70 ring-offset-1 ring-offset-transparent"
                    : "hover:scale-110"
                }`}
                style={{ background: colour }}
              />
            ))}
          </div>
        </Row>

        {monitors.length > 1 && (
          <Row label="Display">
            <select
              value={config.monitor.kind === "index" ? config.monitor.index : -1}
              onChange={(e) => {
                const index = Number(e.target.value);
                patch({
                  monitor: index < 0 ? { kind: "primary" } : { kind: "index", index },
                });
              }}
              className="max-w-[150px] shrink-0 cursor-pointer rounded bg-white/8 px-1.5 py-[2px] text-[10px] outline-none"
            >
              <option value={-1}>Primary</option>
              {monitors.map((monitor) => (
                <option key={monitor.index} value={monitor.index}>
                  {monitor.label}
                </option>
              ))}
            </select>
          </Row>
        )}

        <div className="my-1 h-px bg-white/8" />

        <Row label="Always expanded">
          <Toggle
            label="Always expanded"
            checked={config.alwaysExpanded}
            onChange={(alwaysExpanded) => patch({ alwaysExpanded })}
          />
        </Row>
        <Row label="Click through when resting">
          <Toggle
            label="Click through when resting"
            checked={config.clickThroughWhenCollapsed}
            onChange={(clickThroughWhenCollapsed) =>
              patch({ clickThroughWhenCollapsed })
            }
          />
        </Row>
        <Row label="Peek when an agent needs you">
          <Toggle
            label="Peek when an agent needs you"
            checked={config.peekOnAttention}
            onChange={(peekOnAttention) => patch({ peekOnAttention })}
          />
        </Row>
        <Row label="Alert at 80% and 100%">
          <Toggle
            label="Alert at 80% and 100%"
            checked={config.notifyOnThresholds}
            onChange={(notifyOnThresholds) => patch({ notifyOnThresholds })}
          />
        </Row>
        <Row label="Show resets as countdown">
          <Toggle
            label="Show resets as countdown"
            checked={config.resetAsCountdown}
            onChange={(resetAsCountdown) => patch({ resetAsCountdown })}
          />
        </Row>
        <Row label="Start at sign-in">
          <Toggle
            label="Start at sign-in"
            checked={config.launchAtLogin}
            onChange={(launchAtLogin) => patch({ launchAtLogin })}
          />
        </Row>

        <div className="my-1 h-px bg-white/8" />

        <p className="py-1 text-[10px] text-notch-faint">Providers</p>
        {PROVIDERS.map((provider) => (
          <Row key={provider.id} label={provider.label}>
            <Toggle
              label={provider.label}
              checked={config.providers[provider.id] ?? true}
              onChange={(enabled) =>
                patch({
                  providers: { ...config.providers, [provider.id]: enabled },
                })
              }
            />
          </Row>
        ))}

        <Row label="Ollama address">
          <input
            type="text"
            value={config.ollamaUrl}
            onChange={(e) => patch({ ollamaUrl: e.target.value })}
            spellCheck={false}
            className="w-[150px] shrink-0 rounded bg-white/8 px-1.5 py-[2px] text-[10px] outline-none focus:bg-white/12"
            aria-label="Ollama address"
          />
        </Row>

        <div className="my-1 h-px bg-white/8" />

        <div className="flex gap-1.5 py-1.5">
          <button
            type="button"
            onClick={onOpenConfigDir}
            className="flex flex-1 cursor-pointer items-center justify-center gap-1 rounded bg-white/8 py-1 text-[10px] transition-colors hover:bg-white/14"
          >
            <FolderOpen className="size-3" /> Config folder
          </button>
          <button
            type="button"
            onClick={onQuit}
            className="flex cursor-pointer items-center justify-center gap-1 rounded bg-white/8 px-2 py-1 text-[10px] text-state-bad transition-colors hover:bg-state-bad/20"
          >
            <Power className="size-3" /> Quit
          </button>
        </div>
      </div>
    </div>
  );
}
