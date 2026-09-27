import { createHash } from "node:crypto";
import {
  resumePathForCanonicalField,
  type CanonicalField,
} from "./resume.js";

export type SnapshotNode = {
  role: string;
  name: string;
  description?: string;
  value?: string;
  group_id?: string;
  group_role?: string;
  group_label?: string;
  checked?: string;
  selected?: boolean;
  required?: boolean;
  disabled?: boolean;
  read_only?: boolean;
  element_id?: string;
  actionable: boolean;
};

export type CachedFieldMapping = {
  canonicalField: CanonicalField;
  resumePath: string;
  confidence: number;
  source: "name" | "group" | "resume_upload" | "section";
};

export type ResolvedStructureNode = {
  node: SnapshotNode;
  key: string;
  mapping?: CachedFieldMapping;
};

const MAX_STRUCTURE_NODES = 256;
const MAX_MAPPING_CACHE_ENTRIES = 64;
const mappingCache = new Map<string, Map<string, CachedFieldMapping>>();

export type RepeatSectionKind = "education" | "experience" | "projects";

type SectionField = {
  canonicalField: CanonicalField;
  property: string;
};

export function repeatSectionKind(label: string): RepeatSectionKind | undefined {
  const normalized = normalizeLabel(label);
  if (/^(教育经历|教育背景|education)(\d+)?$/u.test(normalized)) return "education";
  if (
    /^(实习经历|工作经历|工作经验|internship|internshipexperience|workexperience|experience)(\d+)?$/u.test(
      normalized,
    )
  ) {
    return "experience";
  }
  if (
    /^(项目经历|项目经验|project|projects|projectexperience)(\d+)?$/u.test(normalized)
  ) {
    return "projects";
  }
  return undefined;
}

function explicitSectionIndex(label: string): number | undefined {
  const match = normalizeLabel(label).match(/(\d+)$/u);
  if (!match?.[1]) return undefined;
  const number = Number(match[1]);
  return Number.isSafeInteger(number) && number > 0 ? number - 1 : undefined;
}

function matchSectionField(
  kind: RepeatSectionKind,
  label: string,
): SectionField | undefined {
  const normalized = normalizeLabel(label);
  const matches = (...values: string[]) =>
    values.some((value) => normalized === normalizeLabel(value));

  if (kind === "education") {
    if (matches("学校", "院校", "school", "university")) {
      return { canonicalField: "university", property: "school" };
    }
    if (matches("学历", "学位", "degree")) {
      return { canonicalField: "degree", property: "degree" };
    }
    if (matches("专业", "major", "field of study")) {
      return { canonicalField: "major", property: "major" };
    }
    if (matches("入学时间", "开始时间", "start date")) {
      return { canonicalField: "education_start_date", property: "start_date" };
    }
    if (matches("毕业时间", "预计毕业时间", "结束时间", "graduation date", "end date")) {
      return { canonicalField: "graduation_date", property: "graduation_date" };
    }
    if (matches("GPA", "GPA / 绩点", "绩点", "grade point average")) {
      return { canonicalField: "gpa", property: "gpa" };
    }
  }

  if (kind === "experience") {
    if (matches("公司", "公司名称", "company")) {
      return { canonicalField: "current_company", property: "company" };
    }
    if (matches("职位", "岗位", "title", "job title")) {
      return { canonicalField: "current_title", property: "title" };
    }
    if (matches("地点", "工作地点", "location")) {
      return { canonicalField: "experience_location", property: "location" };
    }
    if (matches("开始时间", "start date")) {
      return { canonicalField: "experience_start_date", property: "start_date" };
    }
    if (matches("结束时间", "end date")) {
      return { canonicalField: "experience_end_date", property: "end_date" };
    }
    if (matches("工作内容", "工作职责", "职责", "summary", "description")) {
      return { canonicalField: "experience_summary", property: "summary" };
    }
  }

  if (kind === "projects") {
    if (matches("项目名称", "project name", "name")) {
      return { canonicalField: "project_name", property: "name" };
    }
    if (matches("项目角色", "角色", "project role", "role")) {
      return { canonicalField: "project_role", property: "role" };
    }
    if (matches("开始时间", "start date")) {
      return { canonicalField: "project_start_date", property: "start_date" };
    }
    if (matches("结束时间", "end date")) {
      return { canonicalField: "project_end_date", property: "end_date" };
    }
    if (matches("技术栈", "技术", "technologies", "tech stack")) {
      return { canonicalField: "project_technologies", property: "technologies" };
    }
    if (matches("项目描述", "项目介绍", "summary", "description")) {
      return { canonicalField: "project_summary", property: "summary" };
    }
    if (matches("项目链接", "project url", "url", "link")) {
      return { canonicalField: "project_url", property: "url" };
    }
  }
  return undefined;
}

export function normalizeLabel(value: string): string {
  return value.normalize("NFKC").toLowerCase().replace(/[\s_\-:：/()（）.]+/g, "");
}

const fieldKeywords: ReadonlyArray<{
  field: CanonicalField;
  exact: readonly string[];
  contains: readonly string[];
}> = [
  { field: "first_name", exact: ["firstname", "givenname", "名"], contains: ["firstname", "givenname"] },
  { field: "last_name", exact: ["lastname", "surname", "familyname", "姓"], contains: ["lastname", "surname", "familyname"] },
  { field: "full_name", exact: ["name", "fullname", "姓名"], contains: ["fullname", "candidatename"] },
  { field: "email", exact: ["email", "emailaddress", "邮箱", "电子邮箱"], contains: ["email"] },
  { field: "phone", exact: ["phone", "phonenumber", "mobile", "mobilenumber", "手机号", "手机号码", "联系电话", "电话"], contains: ["phone", "mobile", "手机号", "联系电话"] },
  { field: "city", exact: ["city", "location", "currentlocation", "所在城市", "当前城市", "城市"], contains: ["currentlocation", "所在城市", "当前城市"] },
  { field: "address", exact: ["address", "mailingaddress", "地址", "通讯地址"], contains: ["address", "通讯地址"] },
  { field: "university", exact: ["school", "university", "college", "学校", "院校", "毕业院校"], contains: ["school", "university", "college", "毕业院校"] },
  { field: "degree", exact: ["degree", "educationlevel", "学历", "学位"], contains: ["degree", "educationlevel", "学历", "学位"] },
  { field: "major", exact: ["discipline", "major", "fieldofstudy", "专业"], contains: ["discipline", "major", "fieldofstudy", "专业"] },
  { field: "graduation_date", exact: ["graduationdate", "graduationtime", "enddate", "毕业时间", "预计毕业时间"], contains: ["graduation", "毕业时间"] },
  { field: "gpa", exact: ["gpa", "gradepointaverage", "绩点"], contains: ["gpa", "绩点"] },
  { field: "current_company", exact: ["currentcompany", "company", "当前公司", "公司"], contains: ["currentcompany", "当前公司"] },
  { field: "current_title", exact: ["currenttitle", "jobtitle", "title", "职位", "当前职位"], contains: ["currenttitle", "jobtitle", "当前职位"] },
  { field: "linkedin", exact: ["linkedin", "linkedinurl", "linkedinprofile"], contains: ["linkedin"] },
  { field: "github", exact: ["github", "githuburl"], contains: ["github"] },
  { field: "portfolio", exact: ["portfolio", "website", "personalwebsite", "个人主页", "个人网站"], contains: ["portfolio", "personalwebsite", "个人主页", "个人网站"] },
  { field: "cover_letter", exact: ["coverletter", "additionalinformation", "additionalinfo", "motivation", "selfintroduction", "自我介绍", "补充信息", "求职动机"], contains: ["coverletter", "additionalinformation", "selfintroduction", "自我介绍", "补充信息", "求职动机"] },
  { field: "accept_transfer", exact: ["是否接受岗位调剂", "是否接受调剂", "接受岗位调剂", "accepttransfer", "willingtotransfer"], contains: ["岗位调剂", "接受调剂", "accepttransfer", "willingtotransfer"] },
];

export function matchField(
  label: string,
): { canonicalField: CanonicalField; confidence: number } | undefined {
  const normalized = normalizeLabel(label);
  if (!normalized) return undefined;
  for (const rule of fieldKeywords) {
    if (rule.exact.includes(normalized)) {
      return { canonicalField: rule.field, confidence: 0.99 };
    }
  }
  for (const rule of fieldKeywords) {
    if (rule.contains.some((keyword) => normalized.includes(normalizeLabel(keyword)))) {
      return { canonicalField: rule.field, confidence: 0.88 };
    }
  }
  return undefined;
}

export function isResumeUpload(node: SnapshotNode): boolean {
  const label = normalizeLabel(node.name);
  return (
    label.includes("resume") ||
    label.includes("cv") ||
    label.includes("简历") ||
    label.includes("附件")
  );
}

const choiceAliasGroups = [
  ["是", "yes", "true", "1", "接受", "accept"],
  ["否", "no", "false", "0", "不接受", "decline"],
] as const;

export function choiceMatches(option: string, desired: string): boolean {
  const normalizedOption = normalizeLabel(option);
  const normalizedDesired = normalizeLabel(desired);
  if (!normalizedOption || !normalizedDesired) return false;
  if (normalizedOption === normalizedDesired) return true;
  return choiceAliasGroups.some((group) => {
    const normalized = group.map((item) => normalizeLabel(item));
    return normalized.includes(normalizedOption) && normalized.includes(normalizedDesired);
  });
}

function isStructureNode(node: SnapshotNode): boolean {
  const role = node.role.toLowerCase();
  if (node.actionable && isResumeUpload(node)) return true;
  return [
    "textbox",
    "searchbox",
    "spinbutton",
    "combobox",
    "listbox",
    "radio",
    "checkbox",
    "datetime",
    "date",
    "time",
  ].includes(role);
}

function structuralBase(node: SnapshotNode): string {
  return JSON.stringify([
    normalizeLabel(node.role),
    normalizeLabel(node.name),
    normalizeLabel(node.group_role ?? ""),
    normalizeLabel(node.group_label ?? ""),
    normalizeLabel(node.description ?? ""),
    node.actionable ? 1 : 0,
  ]);
}

function structuralEntries(nodes: readonly SnapshotNode[]): Array<{ node: SnapshotNode; key: string }> {
  const counts = new Map<string, number>();
  const entries: Array<{ node: SnapshotNode; key: string }> = [];
  for (const node of nodes.slice(0, MAX_STRUCTURE_NODES)) {
    if (!isStructureNode(node)) continue;
    const base = structuralBase(node);
    const occurrence = counts.get(base) ?? 0;
    counts.set(base, occurrence + 1);
    entries.push({ node, key: `${base}#${occurrence}` });
  }
  return entries;
}

export function formStructureSignature(nodes: readonly SnapshotNode[]): string {
  const bases = structuralEntries(nodes).map(({ key }) => key);
  return createHash("sha256")
    .update(JSON.stringify(bases))
    .digest("hex")
    .slice(0, 24);
}

function deriveMappings(
  entries: ReadonlyArray<{ node: SnapshotNode; key: string }>,
): Map<string, CachedFieldMapping> {
  const mappings = new Map<string, CachedFieldMapping>();
  const groupIndexes = new Map<string, number>();
  const nextIndexes: Record<RepeatSectionKind, number> = {
    education: 0,
    experience: 0,
    projects: 0,
  };

  for (const { node, key } of entries) {
    const role = node.role.toLowerCase();
    if (node.actionable && isResumeUpload(node)) {
      mappings.set(key, {
        canonicalField: "resume_path",
        resumePath: resumePathForCanonicalField("resume_path"),
        confidence: 1,
        source: "resume_upload",
      });
      continue;
    }

    const sectionKind = node.group_label
      ? repeatSectionKind(node.group_label)
      : undefined;
    if (sectionKind && node.group_label) {
      const explicitIndex = explicitSectionIndex(node.group_label);
      const groupKey =
        node.group_id ?? `${sectionKind}:${normalizeLabel(node.group_label)}`;
      let sectionIndex = explicitIndex ?? groupIndexes.get(groupKey);
      if (sectionIndex === undefined) {
        sectionIndex = nextIndexes[sectionKind];
        nextIndexes[sectionKind] += 1;
      } else {
        nextIndexes[sectionKind] = Math.max(nextIndexes[sectionKind], sectionIndex + 1);
      }
      groupIndexes.set(groupKey, sectionIndex);

      const sectionField = matchSectionField(sectionKind, node.name);
      if (sectionField) {
        mappings.set(key, {
          canonicalField: sectionField.canonicalField,
          resumePath: `${sectionKind}[${sectionIndex}].${sectionField.property}`,
          confidence: 1,
          source: "section",
        });
        continue;
      }
    }

    if ((role === "radio" || role === "checkbox") && node.group_label) {
      const match = matchField(node.group_label);
      if (match) {
        mappings.set(key, {
          ...match,
          resumePath: resumePathForCanonicalField(match.canonicalField),
          source: "group",
        });
      }
      continue;
    }

    const match = matchField(node.name);
    if (match) {
      mappings.set(key, {
        ...match,
        resumePath: resumePathForCanonicalField(match.canonicalField),
        source: "name",
      });
    }
  }
  return mappings;
}

function remember(signature: string, mappings: Map<string, CachedFieldMapping>): void {
  if (mappingCache.has(signature)) {
    mappingCache.delete(signature);
  }
  mappingCache.set(signature, mappings);
  while (mappingCache.size > MAX_MAPPING_CACHE_ENTRIES) {
    const oldest = mappingCache.keys().next().value as string | undefined;
    if (oldest === undefined) break;
    mappingCache.delete(oldest);
  }
}

export function resolveFormMappings(nodes: readonly SnapshotNode[]): {
  signature: string;
  cacheHit: boolean;
  cacheEntries: number;
  nodes: ResolvedStructureNode[];
} {
  const entries = structuralEntries(nodes);
  const signature = createHash("sha256")
    .update(JSON.stringify(entries.map(({ key }) => key)))
    .digest("hex")
    .slice(0, 24);
  const cached = mappingCache.get(signature);
  const cacheHit = cached !== undefined;
  const mappings = cached ?? deriveMappings(entries);
  remember(signature, mappings);
  return {
    signature,
    cacheHit,
    cacheEntries: mappingCache.size,
    nodes: entries.map(({ node, key }) => {
      const mapping = mappings.get(key);
      return mapping === undefined ? { node, key } : { node, key, mapping };
    }),
  };
}
