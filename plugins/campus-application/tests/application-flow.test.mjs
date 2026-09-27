import assert from "node:assert/strict";
import test from "node:test";
import {
  detectStepMarker,
  planApplicationFlow,
} from "../dist/application-flow.js";

function text(name) {
  return { role: "StaticText", name, actionable: false };
}

function button(name, id = "element_button") {
  return { role: "button", name, element_id: id, actionable: true };
}

test("detects explicit Chinese and English step markers", () => {
  assert.equal(
    detectStepMarker([text("第 2 步 / 共 4 步 · 教育经历")]),
    "第 2 步 / 共 4 步 · 教育经历",
  );
  assert.equal(
    detectStepMarker([text("Step 3 of 5 · Experience")]),
    "Step 3 of 5 · Experience",
  );
});

test("plans one next-step click only with explicit workflow context", () => {
  const decision = planApplicationFlow([
    text("第 1 步 / 共 4 步 · 基本信息"),
    button("下一步"),
  ]);

  assert.equal(decision.review_required, false);
  assert.deepEqual(decision.advance, {
    kind: "click",
    intent: "advance_step",
    label: "下一步",
    element_id: "element_button",
    reason:
      "Current step has no remaining fill actions or blockers. Advance one step, then take a fresh Browser snapshot before planning again.",
  });
});

test("does not auto-advance a one-page form without step context", () => {
  const decision = planApplicationFlow([
    button("保存并进入下一步"),
  ]);
  assert.equal(decision.advance, undefined);
  assert.equal(decision.review_required, false);
});

test("final submit always becomes a review boundary", () => {
  const decision = planApplicationFlow([
    text("第 4 步 / 共 4 步 · 确认信息"),
    button("提交申请", "element_submit"),
  ]);

  assert.equal(decision.advance, undefined);
  assert.equal(decision.review_required, true);
  assert.match(decision.review_reason, /Review the completed application/);
});

test("final submit wins over next-like controls", () => {
  const decision = planApplicationFlow([
    text("第 4 步 / 共 4 步 · 确认信息"),
    button("下一步", "element_next"),
    button("提交申请", "element_submit"),
  ]);

  assert.equal(decision.review_required, true);
  assert.equal(decision.advance, undefined);
});
