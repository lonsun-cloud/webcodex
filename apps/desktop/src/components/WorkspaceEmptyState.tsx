import type { ReactNode } from "react";
import { Activity, FileText, Sparkles } from "lucide-react";

export function WorkspaceEmptyState({ message, kind, action }: { message: string; kind: "activity" | "document" | "skill"; action?: ReactNode }) {
  return <div className="workspace-empty-state">
    <span className="workspace-empty-glyph" aria-hidden="true">
      {kind === "activity" && <Activity size={22} strokeWidth={1.75} />}
      {kind === "document" && <FileText size={22} strokeWidth={1.75} />}
      {kind === "skill" && <Sparkles size={22} strokeWidth={1.75} />}
    </span>
    <strong>{message}</strong>
    {action && <div className="workspace-empty-action">{action}</div>}
  </div>;
}
