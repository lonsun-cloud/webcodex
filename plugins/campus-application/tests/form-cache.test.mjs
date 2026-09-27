import assert from "node:assert/strict";
import test from "node:test";
import {
  formStructureSignature,
  resolveFormMappings,
} from "../dist/form-cache.js";

function fixture(elementSuffix, currentName, checked) {
  return [
    {
      role: "textbox",
      name: "姓名",
      value: currentName,
      element_id: `element_name_${elementSuffix}`,
      actionable: true,
    },
    {
      role: "radio",
      name: " 是",
      group_id: `group_${elementSuffix}`,
      group_role: "group",
      group_label: "是否接受岗位调剂",
      checked: "false",
      element_id: `element_yes_${elementSuffix}`,
      actionable: true,
    },
    {
      role: "radio",
      name: " 否",
      group_id: `group_${elementSuffix}`,
      group_role: "group",
      group_label: "是否接受岗位调剂",
      checked,
      element_id: `element_no_${elementSuffix}`,
      actionable: true,
    },
  ];
}

test("structure signature ignores volatile ids, values, and checked state", () => {
  const before = fixture("a", "", "false");
  const after = fixture("b", "示例候选人", "true");
  assert.equal(formStructureSignature(before), formStructureSignature(after));
});

test("structure signature ignores passive group text that reflects current control values", () => {
  const before = [
    ...fixture("passive-a", "", "false"),
    {
      role: "StaticText",
      name: "----",
      group_id: "group_education_a",
      group_role: "group",
      group_label: "教育经历 1",
      actionable: false,
    },
  ];
  const after = [
    ...fixture("passive-b", "示例候选人", "true"),
    {
      role: "StaticText",
      name: "2027",
      group_id: "group_education_b",
      group_role: "group",
      group_label: "教育经历 1",
      actionable: false,
    },
  ];

  assert.equal(formStructureSignature(before), formStructureSignature(after));
});

test("structure signature ignores unrelated actionable buttons and picker affordances", () => {
  const fields = fixture("buttons", "", "false");
  const withAuxiliaryButtons = [
    ...fields,
    { role: "button", name: "保存并进入下一步", actionable: true },
    {
      role: "button",
      name: "显示月份选择器 显示月份选择器",
      group_id: "group_education",
      group_role: "group",
      group_label: "教育经历 1",
      actionable: true,
    },
  ];

  assert.equal(
    formStructureSignature(fields),
    formStructureSignature(withAuxiliaryButtons),
  );
});

test("bounded mapping cache reuses mappings for the same structure", () => {
  const before = fixture("cache-a", "", "false");
  const after = fixture("cache-b", "示例候选人", "true");

  const first = resolveFormMappings(before);
  const second = resolveFormMappings(after);

  assert.equal(first.signature, second.signature);
  assert.equal(second.cacheHit, true);
  assert.ok(second.cacheEntries >= 1);
  assert.equal(
    second.nodes.find(({ node }) => node.name === "姓名")?.mapping?.canonicalField,
    "full_name",
  );
  assert.equal(
    second.nodes.find(({ node }) => node.name.trim() === "否")?.mapping?.canonicalField,
    "accept_transfer",
  );
});

test("repeated section mappings target indexed resume paths", () => {
  const nodes = [
    {
      role: "textbox",
      name: "学校",
      group_id: "group_edu_1",
      group_role: "group",
      group_label: "教育经历 1",
      element_id: "element_edu1_school",
      actionable: true,
    },
    {
      role: "DateTime",
      name: "入学时间",
      group_id: "group_edu_1",
      group_role: "group",
      group_label: "教育经历 1",
      element_id: "element_edu1_start",
      actionable: true,
    },
    {
      role: "textbox",
      name: "学校",
      group_id: "group_edu_2",
      group_role: "group",
      group_label: "教育经历 2",
      element_id: "element_edu2_school",
      actionable: true,
    },
    {
      role: "textbox",
      name: "公司",
      group_id: "group_exp_2",
      group_role: "group",
      group_label: "实习经历 2",
      element_id: "element_exp2_company",
      actionable: true,
    },
    {
      role: "textbox",
      name: "技术栈",
      group_id: "group_project_2",
      group_role: "group",
      group_label: "项目经历 2",
      element_id: "element_project2_tech",
      actionable: true,
    },
  ];

  const resolved = resolveFormMappings(nodes);
  const byElement = new Map(
    resolved.nodes.map(({ node, mapping }) => [node.element_id, mapping]),
  );

  assert.equal(byElement.get("element_edu1_school")?.resumePath, "education[0].school");
  assert.equal(byElement.get("element_edu1_start")?.resumePath, "education[0].start_date");
  assert.equal(byElement.get("element_edu2_school")?.resumePath, "education[1].school");
  assert.equal(byElement.get("element_exp2_company")?.resumePath, "experience[1].company");
  assert.equal(byElement.get("element_project2_tech")?.resumePath, "projects[1].technologies");
  assert.equal(byElement.get("element_project2_tech")?.canonicalField, "project_technologies");
  assert.equal(byElement.get("element_project2_tech")?.source, "section");
});

test("unnumbered repeated groups use stable document-order indexes", () => {
  const nodes = [
    {
      role: "textbox",
      name: "学校",
      group_id: "group_a",
      group_role: "group",
      group_label: "教育经历",
      element_id: "element_school_a",
      actionable: true,
    },
    {
      role: "textbox",
      name: "专业",
      group_id: "group_a",
      group_role: "group",
      group_label: "教育经历",
      element_id: "element_major_a",
      actionable: true,
    },
    {
      role: "textbox",
      name: "学校",
      group_id: "group_b",
      group_role: "group",
      group_label: "教育经历",
      element_id: "element_school_b",
      actionable: true,
    },
  ];

  const resolved = resolveFormMappings(nodes);
  const byElement = new Map(
    resolved.nodes.map(({ node, mapping }) => [node.element_id, mapping]),
  );

  assert.equal(byElement.get("element_school_a")?.resumePath, "education[0].school");
  assert.equal(byElement.get("element_major_a")?.resumePath, "education[0].major");
  assert.equal(byElement.get("element_school_b")?.resumePath, "education[1].school");
});
