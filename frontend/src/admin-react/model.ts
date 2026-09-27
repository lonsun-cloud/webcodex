export type JsonRecord = Record<string, unknown>;
export type DashboardSection = "overview" | "devices" | "projects" | "activity";

export type DashboardView = {
  overview: JsonRecord;
  diagnostics: JsonRecord;
  devices: JsonRecord[];
  projects: JsonRecord[];
  activity: JsonRecord[];
  errors: Record<DashboardSection, string>;
};

export const emptyDashboard: DashboardView = {
  overview: {}, diagnostics: {}, devices: [], projects: [], activity: [],
  errors: { overview: "", devices: "", projects: "", activity: "" },
};

export function record(value: unknown): JsonRecord {
  return value && typeof value === "object" && !Array.isArray(value) ? value as JsonRecord : {};
}

export function list(value: unknown): JsonRecord[] {
  return Array.isArray(value) ? value.map(record) : [];
}

export function display(value: unknown): string {
  if (value == null || value === "") return "—";
  if (typeof value === "object") {
    try { return JSON.stringify(value); } catch { return "—"; }
  }
  return String(value);
}

export function capabilityLabels(value: unknown): string[] {
  if (Array.isArray(value)) return value.filter((item): item is string => typeof item === "string");
  if (value && typeof value === "object") return Object.entries(value as JsonRecord)
    .filter(([, enabled]) => enabled === true).map(([name]) => name).sort();
  return [];
}

export function semanticStatus(value: unknown): "good" | "warning" | "error" | "info" {
  const normalized = display(value).toLowerCase();
  if (/(error|failed|offline|incompatible|mismatch|disabled|blocked|unavailable)/.test(normalized)) return normalized.includes("disabled") ? "warning" : "error";
  if (/(warning|unknown|pending|degraded|limited)/.test(normalized)) return "warning";
  if (/(ok|ready|online|compatible|enabled|healthy|available|true)/.test(normalized)) return "good";
  return "info";
}

/** A failed section retains its last successful data without hiding fresh sibling sections. */
export function mergeDashboard(previous: DashboardView, raw: unknown): DashboardView {
  const data = record(raw);
  const status = record(data.section_status);
  const errors = { ...previous.errors };
  const failed = (section: DashboardSection) => {
    const state = record(status[section]);
    errors[section] = state.status === "error" ? display(state.error || `${section} unavailable`) : "";
    return Boolean(errors[section]);
  };
  const overviewFailed = failed("overview");
  const devicesFailed = failed("devices");
  const projectsFailed = failed("projects");
  const activityFailed = failed("activity");
  return {
    overview: overviewFailed ? previous.overview : record(data.overview),
    diagnostics: overviewFailed ? previous.diagnostics : record(data.diagnostics),
    devices: devicesFailed ? previous.devices : list(data.devices),
    projects: projectsFailed ? previous.projects : list(data.projects),
    activity: activityFailed ? previous.activity : list(data.activity),
    errors,
  };
}
