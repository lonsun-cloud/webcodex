import assert from "node:assert/strict";
import test from "node:test";
import {
  countRepeatedSections,
  planNextSectionExpansion,
} from "../dist/section-lifecycle.js";

function field(groupId, groupLabel, name = "学校") {
  return {
    role: "textbox",
    name,
    group_id: groupId,
    group_role: "group",
    group_label: groupLabel,
    element_id: `element_${groupId}_${name}`,
    actionable: true,
  };
}

function button(name, suffix) {
  return {
    role: "button",
    name,
    element_id: `element_button_${suffix}`,
    actionable: true,
  };
}

test("counts distinct repeated groups rather than child controls", () => {
  const counts = countRepeatedSections([
    field("edu-1", "教育经历 1"),
    field("edu-1", "教育经历 1", "专业"),
    field("edu-2", "教育经历 2"),
    field("exp-1", "实习经历 1", "公司"),
    field("project-1", "项目经历 1", "项目名称"),
  ]);

  assert.deepEqual(counts, {
    education: 2,
    experience: 1,
    projects: 1,
  });
});

test("plans one expansion at a time in deterministic collection order", () => {
  const nodes = [
    field("edu-1", "教育经历 1"),
    field("exp-1", "实习经历 1", "公司"),
    field("project-1", "项目经历 1", "项目名称"),
    button("新增教育经历", "education"),
    button("新增实习经历", "experience"),
    button("新增项目经历", "projects"),
  ];

  const action = planNextSectionExpansion(nodes, {
    education: 2,
    experience: 2,
    projects: 2,
  });

  assert.deepEqual(action, {
    kind: "click",
    collection: "education",
    resume_path: "education[1]",
    label: "新增教育经历",
    element_id: "element_button_education",
    current_count: 1,
    target_count: 2,
    reason:
      "Add one education section, then take a fresh Browser snapshot before planning fields for the new structure.",
  });
});

test("moves to the next collection after a section is present", () => {
  const nodes = [
    field("edu-1", "教育经历 1"),
    field("edu-2", "教育经历 2"),
    field("exp-1", "实习经历 1", "公司"),
    field("project-1", "项目经历 1", "项目名称"),
    button("新增教育经历", "education"),
    button("新增实习经历", "experience"),
    button("新增项目经历", "projects"),
  ];

  const action = planNextSectionExpansion(nodes, {
    education: 2,
    experience: 2,
    projects: 2,
  });

  assert.equal(action?.collection, "experience");
  assert.equal(action?.resume_path, "experience[1]");
});

test("does not invent lifecycle actions when the page has no add-section affordance", () => {
  const action = planNextSectionExpansion(
    [field("edu-1", "教育经历 1")],
    { education: 2, experience: 2, projects: 2 },
  );
  assert.equal(action, undefined);
});

test("returns no expansion when all requested repeated sections exist", () => {
  const nodes = [
    field("edu-1", "教育经历 1"),
    field("edu-2", "教育经历 2"),
    field("exp-1", "实习经历 1", "公司"),
    field("exp-2", "实习经历 2", "公司"),
    field("project-1", "项目经历 1", "项目名称"),
    field("project-2", "项目经历 2", "项目名称"),
    button("新增教育经历", "education"),
    button("新增实习经历", "experience"),
    button("新增项目经历", "projects"),
  ];

  assert.equal(
    planNextSectionExpansion(nodes, {
      education: 2,
      experience: 2,
      projects: 2,
    }),
    undefined,
  );
});
