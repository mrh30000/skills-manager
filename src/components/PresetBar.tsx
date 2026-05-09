import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { cn } from "../utils";
import { ConfirmDialog } from "./ConfirmDialog";
import { computePresetStatus } from "../lib/presetStatus";
import { getScenarioIconOption } from "../lib/scenarioIcons";
import type { ManagedSkill, Scenario } from "../lib/tauri";

const DEACTIVATE_CONFIRM_THRESHOLD = 20;

export interface PresetBarProps {
  presets: Scenario[];
  managedSkills: ManagedSkill[];
  agentKeys: string[];
  existsInWorkspace: (skill: ManagedSkill, agentKey: string) => boolean;
  onAddSkill: (skill: ManagedSkill, agentKey: string) => Promise<void>;
  onRemoveSkill: (skill: ManagedSkill, agentKey: string) => Promise<void>;
  onComplete: () => Promise<void>;
}

export function PresetBar({
  presets,
  managedSkills,
  agentKeys,
  existsInWorkspace,
  onAddSkill,
  onRemoveSkill,
  onComplete,
}: PresetBarProps) {
  const { t } = useTranslation();
  const [activePresetId, setActivePresetId] = useState<string | null>(null);
  const [loadingKey, setLoadingKey] = useState<string | null>(null);
  const [confirmDeactivate, setConfirmDeactivate] = useState<{
    preset: Scenario;
    count: number;
  } | null>(null);
  const [popoverPos, setPopoverPos] = useState<{ top: number; left: number } | null>(null);
  const pillRefs = useRef<Map<string, HTMLButtonElement>>(new Map());
  const popoverRef = useRef<HTMLDivElement>(null);

  const statuses = useMemo(() => {
    const map = new Map<string, ReturnType<typeof computePresetStatus>>();
    for (const preset of presets) {
      map.set(preset.id, computePresetStatus(preset, managedSkills, agentKeys, existsInWorkspace));
    }
    return map;
  }, [presets, managedSkills, agentKeys, existsInWorkspace]);

  const visiblePresets = useMemo(
    () => presets.filter((p) => statuses.get(p.id)?.status !== "empty"),
    [presets, statuses]
  );

  const activePreset = useMemo(
    () => visiblePresets.find((p) => p.id === activePresetId) ?? null,
    [activePresetId, visiblePresets]
  );

  useEffect(() => {
    if (!activePresetId) return;
    const onMouseDown = (e: MouseEvent) => {
      const target = e.target as Node;
      if (popoverRef.current?.contains(target)) return;
      if (pillRefs.current.get(activePresetId)?.contains(target)) return;
      setActivePresetId(null);
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setActivePresetId(null);
    };
    document.addEventListener("mousedown", onMouseDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mousedown", onMouseDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [activePresetId]);

  const openPopover = useCallback((preset: Scenario, btn: HTMLButtonElement) => {
    const rect = btn.getBoundingClientRect();
    const popoverWidth = 272;
    const left = Math.min(rect.left, window.innerWidth - popoverWidth - 16);
    setPopoverPos({ top: rect.bottom + 6, left });
    setActivePresetId((prev) => (prev === preset.id ? null : preset.id));
  }, []);

  const handleActivate = useCallback(async (preset: Scenario) => {
    const key = `${preset.id}-add`;
    setLoadingKey(key);
    const presetSkills = managedSkills.filter((s) => s.scenario_ids.includes(preset.id));
    let added = 0, skipped = 0, failed = 0;
    for (const skill of presetSkills) {
      for (const agentKey of agentKeys) {
        if (existsInWorkspace(skill, agentKey)) { skipped++; continue; }
        try { await onAddSkill(skill, agentKey); added++; }
        catch { failed++; }
      }
    }
    await onComplete();
    if (added > 0) {
      toast.success(t("presetActions.addedToast", { added, skipped }));
    } else if (failed === 0) {
      toast.info(t("presetActions.nothingToAdd"));
    }
    if (failed > 0) toast.error(t("presetActions.partialFailedToast", { count: failed }));
    setLoadingKey(null);
    setActivePresetId(null);
  }, [agentKeys, existsInWorkspace, managedSkills, onAddSkill, onComplete, t]);

  const handleDeactivateConfirmed = useCallback(async (preset: Scenario) => {
    const key = `${preset.id}-remove`;
    setLoadingKey(key);
    setConfirmDeactivate(null);
    const presetSkills = managedSkills.filter((s) => s.scenario_ids.includes(preset.id));
    let removed = 0, failed = 0;
    for (const skill of presetSkills) {
      for (const agentKey of agentKeys) {
        if (!existsInWorkspace(skill, agentKey)) continue;
        try { await onRemoveSkill(skill, agentKey); removed++; }
        catch { failed++; }
      }
    }
    await onComplete();
    if (removed > 0) {
      toast.success(t("presetActions.removedToast", { removed }));
    } else if (failed === 0) {
      toast.info(t("presetActions.nothingToRemove"));
    }
    if (failed > 0) toast.error(t("presetActions.partialFailedToast", { count: failed }));
    setLoadingKey(null);
    setActivePresetId(null);
  }, [agentKeys, existsInWorkspace, managedSkills, onComplete, onRemoveSkill, t]);

  const handleDeactivate = useCallback((preset: Scenario) => {
    const presetSkills = managedSkills.filter((s) => s.scenario_ids.includes(preset.id));
    let count = 0;
    for (const skill of presetSkills) {
      for (const agentKey of agentKeys) {
        if (existsInWorkspace(skill, agentKey)) count++;
      }
    }
    if (count >= DEACTIVATE_CONFIRM_THRESHOLD) {
      setConfirmDeactivate({ preset, count });
    } else {
      handleDeactivateConfirmed(preset);
    }
  }, [agentKeys, existsInWorkspace, handleDeactivateConfirmed, managedSkills]);

  const handlePillClick = useCallback((preset: Scenario, e: React.MouseEvent<HTMLButtonElement>) => {
    const s = statuses.get(preset.id);
    if (!s || loadingKey) return;
    if (e.shiftKey) {
      if (s.status === "active") handleDeactivate(preset);
      else handleActivate(preset);
    } else {
      openPopover(preset, e.currentTarget);
    }
  }, [handleActivate, handleDeactivate, loadingKey, openPopover, statuses]);

  if (visiblePresets.length === 0) return null;

  const busy = loadingKey !== null;

  return (
    <>
      <div className="mb-3 -mt-2">
        <div className="flex items-center gap-1.5 overflow-x-auto scrollbar-hide pb-0.5">
          {visiblePresets.map((preset) => {
            const s = statuses.get(preset.id)!;
            const scenarioIcon = getScenarioIconOption(preset);
            const Icon = scenarioIcon.icon;
            const isOpen = activePresetId === preset.id;
            const isLoading = loadingKey?.startsWith(preset.id) ?? false;

            return (
              <button
                key={preset.id}
                ref={(el) => {
                  if (el) pillRefs.current.set(preset.id, el);
                  else pillRefs.current.delete(preset.id);
                }}
                onClick={(e) => handlePillClick(preset, e)}
                disabled={busy}
                title={t("presetBar.shiftClickHint", { name: preset.name })}
                className={cn(
                  "inline-flex shrink-0 items-center gap-1.5 rounded-full border px-2.5 py-1 text-[12px] font-medium transition-colors disabled:opacity-50",
                  s.status === "active"
                    ? `${scenarioIcon.activeClass} ${scenarioIcon.colorClass} border-transparent`
                    : isOpen
                    ? "border-border bg-surface-hover text-secondary"
                    : "border-border-subtle text-muted hover:border-border hover:text-secondary"
                )}
              >
                {isLoading
                  ? <Loader2 className="h-3 w-3 animate-spin" />
                  : <Icon className="h-3 w-3" />}
                <span className="max-w-[160px] truncate">{preset.name}</span>
                {s.status === "partial" && (
                  <span className="rounded-full bg-amber-500/15 px-1.5 py-px text-[10px] text-amber-500">
                    {s.installed}/{s.total}
                  </span>
                )}
              </button>
            );
          })}
        </div>
      </div>

      {activePreset && popoverPos && createPortal(
        <div
          ref={popoverRef}
          style={{ top: popoverPos.top, left: popoverPos.left }}
          className="fixed z-50 w-68 rounded-xl border border-border bg-bg-secondary p-4 shadow-xl"
        >
          {(() => {
            const s = statuses.get(activePreset.id)!;
            const scenarioIcon = getScenarioIconOption(activePreset);
            const Icon = scenarioIcon.icon;
            const addLoading = loadingKey === `${activePreset.id}-add`;
            const rmLoading = loadingKey === `${activePreset.id}-remove`;

            return (
              <>
                <div className="mb-3 flex items-center gap-2.5">
                  <span className={cn(
                    "flex h-7 w-7 shrink-0 items-center justify-center rounded-md border",
                    s.status === "active"
                      ? `${scenarioIcon.activeClass} ${scenarioIcon.colorClass}`
                      : "border-border bg-surface text-muted"
                  )}>
                    <Icon className="h-3.5 w-3.5" />
                  </span>
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-[13px] font-semibold text-primary">{activePreset.name}</p>
                    <p className="text-[12px] text-muted">
                      {t("presetBar.skillCount", { count: activePreset.skill_count })}
                    </p>
                  </div>
                  <span className={cn(
                    "shrink-0 rounded-full px-2 py-0.5 text-[11px] font-medium",
                    s.status === "active" && "bg-emerald-500/10 text-emerald-500",
                    s.status === "partial" && "bg-amber-500/10 text-amber-500",
                    s.status === "inactive" && "bg-surface-hover text-muted"
                  )}>
                    {s.status === "active" && t("presetActions.statusActive")}
                    {s.status === "partial" && t("presetActions.statusPartial", { installed: s.installed, total: s.total })}
                    {s.status === "inactive" && t("presetActions.statusInactive")}
                  </span>
                </div>

                <div className="flex items-center gap-2">
                  {(s.status === "inactive" || s.status === "partial") && (
                    <button
                      onClick={() => handleActivate(activePreset)}
                      disabled={busy}
                      className="flex-1 inline-flex items-center justify-center gap-1 rounded-md bg-accent px-3 py-1.5 text-[12px] font-medium text-white transition-colors hover:bg-accent-hover disabled:opacity-50"
                    >
                      {addLoading && <Loader2 className="h-3 w-3 animate-spin" />}
                      {t("presetActions.activate")}
                    </button>
                  )}
                  {(s.status === "active" || s.status === "partial") && (
                    <button
                      onClick={() => handleDeactivate(activePreset)}
                      disabled={busy}
                      className="flex-1 inline-flex items-center justify-center gap-1 rounded-md border border-border-subtle px-3 py-1.5 text-[12px] font-medium text-muted transition-colors hover:border-red-400 hover:text-red-400 disabled:opacity-50"
                    >
                      {rmLoading && <Loader2 className="h-3 w-3 animate-spin" />}
                      {t("presetActions.deactivate")}
                    </button>
                  )}
                </div>
              </>
            );
          })()}
        </div>,
        document.body
      )}

      {confirmDeactivate && (
        <ConfirmDialog
          open
          title={t("presetActions.deactivate")}
          message={t("presetBar.deactivateConfirm", {
            name: confirmDeactivate.preset.name,
            count: confirmDeactivate.count,
          })}
          tone="danger"
          onClose={() => setConfirmDeactivate(null)}
          onConfirm={() => handleDeactivateConfirmed(confirmDeactivate.preset)}
        />
      )}
    </>
  );
}
