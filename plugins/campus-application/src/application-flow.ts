import {
  normalizeLabel,
  type SnapshotNode,
} from "./form-cache.js";

export type FlowAction = {
  kind: "click";
  intent: "advance_step";
  label: string;
  element_id: string;
  reason: string;
};

export type ApplicationFlowDecision = {
  step_marker: string;
  advance?: FlowAction;
  review_required: boolean;
  review_reason: string;
};

const nextAliases = [
  "下一步",
  "下一页",
  "保存并下一步",
  "保存并进入下一步",
  "保存并继续",
  "继续",
  "继续填写",
  "next",
  "next step",
  "continue",
  "save and continue",
  "save and next",
] as const;

const submitAliases = [
  "提交申请",
  "提交",
  "确认提交",
  "正式提交",
  "submit application",
  "submit",
  "final submit",
] as const;

function matchesAlias(label: string, aliases: readonly string[]): boolean {
  const normalized = normalizeLabel(label);
  return aliases.some((alias) => normalized === normalizeLabel(alias));
}

function isButton(node: SnapshotNode): boolean {
  return (
    node.actionable &&
    node.role.toLowerCase() === "button" &&
    Boolean(node.element_id)
  );
}

export function detectStepMarker(nodes: readonly SnapshotNode[]): string | undefined {
  for (const node of nodes) {
    const label = node.name.trim();
    if (!label) continue;
    const normalized = normalizeLabel(label);
    if (
      /第\d+步/u.test(normalized) ||
      /步骤\d+/u.test(normalized) ||
      /step\d+(?:of\d+)?/u.test(normalized)
    ) {
      return label.slice(0, 200);
    }
  }
  return undefined;
}

function findSubmitButton(nodes: readonly SnapshotNode[]): SnapshotNode | undefined {
  return nodes.find(
    (node) => isButton(node) && matchesAlias(node.name, submitAliases),
  );
}

function findNextButton(nodes: readonly SnapshotNode[]): SnapshotNode | undefined {
  return nodes.find(
    (node) => isButton(node) && matchesAlias(node.name, nextAliases),
  );
}

export function planApplicationFlow(
  nodes: readonly SnapshotNode[],
): ApplicationFlowDecision {
  const stepMarker = detectStepMarker(nodes) ?? "";
  const submit = findSubmitButton(nodes);
  if (submit) {
    return {
      step_marker: stepMarker,
      review_required: true,
      review_reason:
        `Final submission control "${submit.name}" is present. Review the completed application before any submit action.`,
    };
  }

  const next = findNextButton(nodes);
  if (next?.element_id && stepMarker) {
    return {
      step_marker: stepMarker,
      advance: {
        kind: "click",
        intent: "advance_step",
        label: next.name,
        element_id: next.element_id,
        reason:
          "Current step has no remaining fill actions or blockers. Advance one step, then take a fresh Browser snapshot before planning again.",
      },
      review_required: false,
      review_reason: "",
    };
  }

  return {
    step_marker: stepMarker,
    review_required: false,
    review_reason: "",
  };
}
