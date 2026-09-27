import {
  definePlugin,
  defineTool,
  runPlugin,
  schema,
  textResult,
} from "@yyjeqhc/webcodex-plugin-sdk";
import {
  choiceMatches,
  isResumeUpload,
  matchField,
  normalizeLabel,
  resolveFormMappings,
  type SnapshotNode,
} from "./form-cache.js";
import {
  canonicalFields,
  canonicalProfileFromResume,
  canonicalProfileSchema,
  loadResumeProfile,
  resolveResumeValue,
  resumeProfileSchema,
  type CanonicalField,
  type ResumeProfile,
} from "./resume.js";
import {
  planNextSectionExpansion,
  type SectionExpansionAction,
} from "./section-lifecycle.js";
import {
  planApplicationFlow,
  type FlowAction,
} from "./application-flow.js";

const nodeSchema = schema.object({
  role: schema.string({ maxLength: 80 }),
  name: schema.string({ maxLength: 1200 }),
  description: schema.optional(schema.string({ maxLength: 1200 })),
  value: schema.optional(schema.string({ maxLength: 4096 })),
  group_id: schema.optional(schema.string({ maxLength: 160 })),
  group_role: schema.optional(schema.string({ maxLength: 80 })),
  group_label: schema.optional(schema.string({ maxLength: 1200 })),
  checked: schema.optional(schema.string({ maxLength: 32 })),
  selected: schema.optional(schema.boolean()),
  required: schema.optional(schema.boolean()),
  disabled: schema.optional(schema.boolean()),
  read_only: schema.optional(schema.boolean()),
  element_id: schema.optional(schema.string({ maxLength: 160 })),
  actionable: schema.boolean(),
});

const recognizedSchema = schema.object({
  canonical_field: schema.string({ enum: canonicalFields }),
  resume_path: schema.string({ maxLength: 300 }),
  label: schema.string({ maxLength: 1200 }),
  role: schema.string({ maxLength: 80 }),
  element_id: schema.string({ maxLength: 160 }),
  current_value: schema.string({ maxLength: 4096 }),
  proposed_value: schema.string({ maxLength: 4096 }),
  confidence: schema.number(),
  support: schema.string({
    enum: ["input_text", "select_option", "set_value", "upload_file", "click", "manual_review"] as const,
  }),
});

const blockerSchema = schema.object({
  label: schema.string({ maxLength: 1200 }),
  role: schema.string({ maxLength: 80 }),
  element_id: schema.string({ maxLength: 160 }),
  reason: schema.string({ maxLength: 500 }),
});

const actionSchema = schema.object({
  kind: schema.string({
    enum: ["input_text", "select_option", "set_value", "upload_file", "click", "manual_review"] as const,
  }),
  canonical_field: schema.string({ enum: canonicalFields }),
  resume_path: schema.string({ maxLength: 300 }),
  label: schema.string({ maxLength: 1200 }),
  element_id: schema.string({ maxLength: 160 }),
  value: schema.string({ maxLength: 4096 }),
  confidence: schema.number(),
  reason: schema.string({ maxLength: 500 }),
});

const sectionActionSchema = schema.object({
  kind: schema.string({ enum: ["click"] as const }),
  collection: schema.string({ enum: ["education", "experience", "projects"] as const }),
  resume_path: schema.string({ maxLength: 300 }),
  label: schema.string({ maxLength: 1200 }),
  element_id: schema.string({ maxLength: 160 }),
  current_count: schema.integer(),
  target_count: schema.integer(),
  reason: schema.string({ maxLength: 500 }),
});

const flowActionSchema = schema.object({
  kind: schema.string({ enum: ["click"] as const }),
  intent: schema.string({ enum: ["advance_step"] as const }),
  label: schema.string({ maxLength: 1200 }),
  element_id: schema.string({ maxLength: 160 }),
  reason: schema.string({ maxLength: 500 }),
});

type FillSupport =
  | "input_text"
  | "select_option"
  | "set_value"
  | "upload_file"
  | "click"
  | "manual_review";

function supportFor(node: SnapshotNode): FillSupport {
  const role = node.role.toLowerCase();
  if (isResumeUpload(node)) return "upload_file";
  if (role === "combobox" || role === "listbox") return "select_option";
  if (role === "datetime" || role === "date" || role === "time") return "set_value";
  if (role === "textbox" || role === "searchbox" || role === "spinbutton") return "input_text";
  return "manual_review";
}

function adaptValue(
  siteKind: "greenhouse-like" | "lever-like" | "campus-cn-like" | "generic",
  field: CanonicalField,
  value: string,
): string {
  if (siteKind === "campus-cn-like" && field === "degree") {
    const degreeMap: Record<string, string> = {
      Bachelor: "本科",
      Master: "硕士",
      PhD: "博士",
    };
    return degreeMap[value] ?? value;
  }
  return value;
}

function classifySite(title: string, url: string, nodes: readonly SnapshotNode[]) {
  const raw = [
    title,
    url,
    ...nodes.flatMap((node) => [node.name, node.group_label ?? ""]),
  ].join(" ");
  const haystack = normalizeLabel(raw);
  const mappedLanguageEvidence = [
    title,
    ...nodes
      .flatMap((node) => [node.name, node.group_label ?? ""])
      .filter((label) => matchField(label)),
  ].join(" ");
  if (/[㐀-鿿]/u.test(mappedLanguageEvidence)) return "campus-cn-like" as const;
  if (haystack.includes("firstname") && haystack.includes("lastname") && (haystack.includes("school") || haystack.includes("degree"))) {
    return "greenhouse-like" as const;
  }
  if ((haystack.includes("fullname") || haystack.includes("name")) && (haystack.includes("linkedin") || haystack.includes("currentcompany"))) {
    return "lever-like" as const;
  }
  return "generic" as const;
}

function analyzeNodes(nodes: readonly SnapshotNode[], resume: ResumeProfile) {
  const recognized: Array<{
    canonical_field: CanonicalField;
    resume_path: string;
    label: string;
    role: string;
    element_id: string;
    current_value: string;
    proposed_value: string;
    confidence: number;
    support: FillSupport;
  }> = [];
  const blockers: Array<{ label: string; role: string; element_id: string; reason: string }> = [];
  const resolved = resolveFormMappings(nodes);

  for (const { node, mapping } of resolved.nodes) {
    const role = node.role.toLowerCase();
    const resolvedValue = mapping
      ? resolveResumeValue(resume, mapping.resumePath)
      : { found: false, value: "" };

    if (mapping && !resolvedValue.found) {
      blockers.push({
        label: node.group_label || node.name,
        role: node.role,
        element_id: node.element_id ?? "",
        reason: `Mapped resume path ${mapping.resumePath} is unavailable in the structured resume resource.`,
      });
      continue;
    }

    if (
      mapping?.source === "group" &&
      (role === "radio" || role === "checkbox") &&
      node.group_label
    ) {
      const desired = resolvedValue.value;
      if (choiceMatches(node.name, desired)) {
        const checked = node.checked === "true" || node.selected === true;
        recognized.push({
          canonical_field: mapping.canonicalField,
          resume_path: mapping.resumePath,
          label: node.group_label,
          role: node.role,
          element_id: node.element_id ?? "",
          current_value: checked ? desired : "",
          proposed_value: desired,
          confidence: Math.min(1, mapping.confidence + 0.01),
          support: "click",
        });
      }
      continue;
    }

    if (mapping?.source === "resume_upload") {
      recognized.push({
        canonical_field: mapping.canonicalField,
        resume_path: mapping.resumePath,
        label: node.name,
        role: node.role,
        element_id: node.element_id ?? "",
        current_value: node.value ?? "",
        proposed_value: resolvedValue.value,
        confidence: mapping.confidence,
        support: "upload_file",
      });
      continue;
    }

    if (!node.actionable) {
      if (mapping && ["datetime", "date", "time"].includes(role)) {
        recognized.push({
          canonical_field: mapping.canonicalField,
          resume_path: mapping.resumePath,
          label: node.name,
          role: node.role,
          element_id: "",
          current_value: node.value ?? "",
          proposed_value: resolvedValue.value,
          confidence: mapping.confidence,
          support: "manual_review",
        });
        blockers.push({
          label: node.name,
          role: node.role,
          element_id: "",
          reason: "The resume field is visible in the semantic snapshot but has no actionable Browser element identity.",
        });
      }
      continue;
    }

    if (!mapping) {
      if (!["button", "link", "option", "radio", "checkbox"].includes(role)) {
        blockers.push({
          label: node.name,
          role: node.role,
          element_id: node.element_id ?? "",
          reason: "Actionable control is not mapped to the current resume schema.",
        });
      }
      continue;
    }

    const support = supportFor(node);
    recognized.push({
      canonical_field: mapping.canonicalField,
      resume_path: mapping.resumePath,
      label: node.name,
      role: node.role,
      element_id: node.element_id ?? "",
      current_value: node.value ?? "",
      proposed_value: resolvedValue.value,
      confidence: mapping.confidence,
      support,
    });
    if (support === "manual_review") {
      blockers.push({
        label: node.name,
        role: node.role,
        element_id: node.element_id ?? "",
        reason: "The field is recognized, but its control semantics still require human review.",
      });
    }
  }

  return {
    recognized,
    blockers,
    structure_signature: resolved.signature,
    mapping_cache_hit: resolved.cacheHit,
    mapping_cache_entries: resolved.cacheEntries,
  };
}
const profileGet = defineTool({
  name: "profile_get",
  description: "Return the configured structured resume resource plus the bounded canonical fill view used by campus-application. Read-only; never controls a browser or submits an application.",
  inputSchema: schema.object({}),
  outputSchema: schema.object({
    resume: resumeProfileSchema,
    profile: canonicalProfileSchema,
  }),
  annotations: { readOnlyHint: true, destructiveHint: false, idempotentHint: true, openWorldHint: false },
  async execute() {
    const resume = loadResumeProfile();
    const profile = canonicalProfileFromResume(resume);
    return textResult("Loaded configured campus-application resume profile and canonical fill view.", {
      resume,
      profile,
    });
  },
});

const analyzeForm = defineTool({
  name: "analyze_form",
  description: "Analyze a bounded Browser semantic snapshot, classify the ATS-like form, reuse or derive a structure-signature mapping, and report mapped controls plus blockers. It never controls the browser.",
  inputSchema: schema.object({
    title: schema.string({ maxLength: 1000 }),
    url: schema.string({ maxLength: 4000 }),
    nodes: schema.array(nodeSchema, { maxItems: 256 }),
  }),
  outputSchema: schema.object({
    site_kind: schema.string({ enum: ["greenhouse-like", "lever-like", "campus-cn-like", "generic"] as const }),
    recognized: schema.array(recognizedSchema, { maxItems: 256 }),
    blockers: schema.array(blockerSchema, { maxItems: 256 }),
    structure_signature: schema.string({ maxLength: 64 }),
    mapping_cache_hit: schema.boolean(),
    mapping_cache_entries: schema.integer(),
  }),
  annotations: { readOnlyHint: true, destructiveHint: false, idempotentHint: true, openWorldHint: false },
  async execute({ title, url, nodes }) {
    const resume = loadResumeProfile();
    const analysis = analyzeNodes(nodes, resume);
    const site_kind = classifySite(title, url, nodes);
    return textResult(
      `Detected ${site_kind}: ${analysis.recognized.length} mapped controls, ${analysis.blockers.length} blockers; mapping cache ${analysis.mapping_cache_hit ? "hit" : "miss"}.`,
      { site_kind, ...analysis },
    );
  },
});

const planFill = defineTool({
  name: "plan_fill",
  description: "Convert a bounded Browser semantic snapshot into the next safe application step. If the structured resume has more repeated education/experience/project entries than the page currently exposes and a matching add-section button is available, return exactly one section-expansion click and require a fresh snapshot before field filling. Otherwise return the normal bounded fill plan. It never executes browser actions or submits a form.",
  inputSchema: schema.object({
    title: schema.string({ maxLength: 1000 }),
    url: schema.string({ maxLength: 4000 }),
    nodes: schema.array(nodeSchema, { maxItems: 256 }),
  }),
  outputSchema: schema.object({
    site_kind: schema.string({ enum: ["greenhouse-like", "lever-like", "campus-cn-like", "generic"] as const }),
    phase: schema.string({ enum: ["expand_sections", "fill_fields", "advance_step", "ready_for_review"] as const }),
    section_actions: schema.array(sectionActionSchema, { maxItems: 1 }),
    flow_actions: schema.array(flowActionSchema, { maxItems: 1 }),
    step_marker: schema.string({ maxLength: 200 }),
    review_required: schema.boolean(),
    review_reason: schema.string({ maxLength: 500 }),
    actions: schema.array(actionSchema, { maxItems: 256 }),
    blockers: schema.array(blockerSchema, { maxItems: 256 }),
    blocker_count: schema.integer(),
    structure_signature: schema.string({ maxLength: 64 }),
    mapping_cache_hit: schema.boolean(),
    mapping_cache_entries: schema.integer(),
  }),
  annotations: { readOnlyHint: true, destructiveHint: false, idempotentHint: true, openWorldHint: false },
  async execute({ title, url, nodes }) {
    const resume = loadResumeProfile();
    const analysis = analyzeNodes(nodes, resume);
    const site_kind = classifySite(title, url, nodes);
    const flowContext = planApplicationFlow(nodes);
    const sectionExpansion = planNextSectionExpansion(nodes, {
      education: resume.education.length,
      experience: resume.experience.length,
      projects: resume.projects.length,
    });
    if (sectionExpansion) {
      return textResult(
        `Expand ${sectionExpansion.collection} from ${sectionExpansion.current_count} to ${sectionExpansion.target_count}; take a fresh Browser snapshot before filling fields.`,
        {
          site_kind,
          phase: "expand_sections",
          section_actions: [sectionExpansion] as SectionExpansionAction[],
          flow_actions: [] as FlowAction[],
          step_marker: flowContext.step_marker,
          review_required: false,
          review_reason: "",
          actions: [] as never[],
          blockers: [] as Array<{ label: string; role: string; element_id: string; reason: string }>,
          blocker_count: 0,
          structure_signature: analysis.structure_signature,
          mapping_cache_hit: analysis.mapping_cache_hit,
          mapping_cache_entries: analysis.mapping_cache_entries,
        },
      );
    }

    const actions = analysis.recognized
      .map((item) => ({
        ...item,
        proposed_value: adaptValue(site_kind, item.canonical_field, item.proposed_value),
      }))
      .filter((item) => {
        const expectedFileName = item.proposed_value.split(/[\\/]/).pop() ?? item.proposed_value;
        const alreadySatisfied =
          item.current_value === item.proposed_value ||
          (item.support === "upload_file" && item.current_value === expectedFileName);
        return (
          item.support !== "manual_review" &&
          item.proposed_value.length > 0 &&
          !alreadySatisfied
        );
      })
      .map((item) => ({
        kind: item.support,
        canonical_field: item.canonical_field,
        resume_path: item.resume_path,
        label: item.label,
        element_id: item.element_id,
        value: item.proposed_value,
        confidence: item.confidence,
        reason:
          item.support === "input_text"
            ? "Use Browser input_text."
            : item.support === "select_option"
              ? "Use Browser select_option with the exact native value or visible label."
              : item.support === "set_value"
                ? "Use Browser set_value for the native structured form value."
                : item.support === "click"
                  ? "Use Browser click on the option whose group semantics and label match the desired profile choice."
                  : "Use Browser upload_file with the authorized resume project and this project-relative path.",
      }));
    if (actions.length === 0 && analysis.blockers.length === 0) {
      const flow = flowContext;
      if (flow.review_required) {
        return textResult(
          flow.review_reason,
          {
            site_kind,
            phase: "ready_for_review",
            section_actions: [] as SectionExpansionAction[],
            flow_actions: [] as FlowAction[],
            step_marker: flow.step_marker,
            review_required: true,
            review_reason: flow.review_reason,
            actions: [] as never[],
            blockers: [] as Array<{ label: string; role: string; element_id: string; reason: string }>,
            blocker_count: 0,
            structure_signature: analysis.structure_signature,
            mapping_cache_hit: analysis.mapping_cache_hit,
            mapping_cache_entries: analysis.mapping_cache_entries,
          },
        );
      }
      if (flow.advance) {
        return textResult(
          `Current step is complete. Advance with "${flow.advance.label}", then take a fresh Browser snapshot before planning again.`,
          {
            site_kind,
            phase: "advance_step",
            section_actions: [] as SectionExpansionAction[],
            flow_actions: [flow.advance] as FlowAction[],
            step_marker: flow.step_marker,
            review_required: false,
            review_reason: "",
            actions: [] as never[],
            blockers: [] as Array<{ label: string; role: string; element_id: string; reason: string }>,
            blocker_count: 0,
            structure_signature: analysis.structure_signature,
            mapping_cache_hit: analysis.mapping_cache_hit,
            mapping_cache_entries: analysis.mapping_cache_entries,
          },
        );
      }
    }

    return textResult(
      `Prepared ${actions.length} proposed actions for ${site_kind}; ${analysis.blockers.length} blockers remain; mapping cache ${analysis.mapping_cache_hit ? "hit" : "miss"}.`,
      {
        site_kind,
        phase: "fill_fields",
        section_actions: [] as SectionExpansionAction[],
        flow_actions: [] as FlowAction[],
        step_marker: flowContext.step_marker,
        review_required: false,
        review_reason: "",
        actions,
        blockers: analysis.blockers,
        blocker_count: analysis.blockers.length,
        structure_signature: analysis.structure_signature,
        mapping_cache_hit: analysis.mapping_cache_hit,
        mapping_cache_entries: analysis.mapping_cache_entries,
      },
    );
  },
});

runPlugin(definePlugin({ tools: [profileGet, analyzeForm, planFill] }));
