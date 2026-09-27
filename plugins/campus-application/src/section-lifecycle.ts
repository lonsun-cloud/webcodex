import {
  normalizeLabel,
  repeatSectionKind,
  type RepeatSectionKind,
  type SnapshotNode,
} from "./form-cache.js";

export type SectionExpansionAction = {
  kind: "click";
  collection: RepeatSectionKind;
  resume_path: string;
  label: string;
  element_id: string;
  current_count: number;
  target_count: number;
  reason: string;
};

export type SectionCounts = Record<RepeatSectionKind, number>;

const sectionOrder: readonly RepeatSectionKind[] = [
  "education",
  "experience",
  "projects",
];

const addButtonAliases: Record<RepeatSectionKind, readonly string[]> = {
  education: [
    "新增教育经历",
    "添加教育经历",
    "添加教育背景",
    "新增教育背景",
    "add education",
    "add another education",
  ],
  experience: [
    "新增实习经历",
    "添加实习经历",
    "新增工作经历",
    "添加工作经历",
    "add internship",
    "add experience",
    "add work experience",
    "add another experience",
  ],
  projects: [
    "新增项目经历",
    "添加项目经历",
    "新增项目",
    "添加项目",
    "add project",
    "add another project",
  ],
};

function matchesAlias(label: string, aliases: readonly string[]): boolean {
  const normalized = normalizeLabel(label);
  return aliases.some((alias) => normalized === normalizeLabel(alias));
}

export function countRepeatedSections(nodes: readonly SnapshotNode[]): SectionCounts {
  const groups: Record<RepeatSectionKind, Set<string>> = {
    education: new Set<string>(),
    experience: new Set<string>(),
    projects: new Set<string>(),
  };

  for (const node of nodes) {
    if (!node.group_label) continue;
    const kind = repeatSectionKind(node.group_label);
    if (!kind) continue;
    groups[kind].add(
      node.group_id ?? `${kind}:${normalizeLabel(node.group_label)}`,
    );
  }

  return {
    education: groups.education.size,
    experience: groups.experience.size,
    projects: groups.projects.size,
  };
}

function findAddButton(
  nodes: readonly SnapshotNode[],
  kind: RepeatSectionKind,
): SnapshotNode | undefined {
  return nodes.find((node) => {
    if (!node.actionable || node.role.toLowerCase() !== "button") return false;
    if (!node.element_id) return false;
    return matchesAlias(node.name, addButtonAliases[kind]);
  });
}

export function planNextSectionExpansion(
  nodes: readonly SnapshotNode[],
  targets: SectionCounts,
): SectionExpansionAction | undefined {
  const current = countRepeatedSections(nodes);

  for (const kind of sectionOrder) {
    if (current[kind] >= targets[kind]) continue;
    const button = findAddButton(nodes, kind);
    if (!button?.element_id) continue;

    return {
      kind: "click",
      collection: kind,
      resume_path: `${kind}[${current[kind]}]`,
      label: button.name,
      element_id: button.element_id,
      current_count: current[kind],
      target_count: targets[kind],
      reason:
        `Add one ${kind} section, then take a fresh Browser snapshot before planning fields for the new structure.`,
    };
  }

  return undefined;
}
