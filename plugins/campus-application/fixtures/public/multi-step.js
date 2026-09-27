const state = {
  step: 1,
  education: 0,
  experience: 0,
  projects: 0,
};

const totalSteps = 4;
const progress = document.querySelector("#wizard-progress");

function educationMarkup(index) {
  return `<fieldset class="repeat-card"><legend>教育经历 ${index}</legend><div class="grid">
    <div><label for="ms-edu${index}-school">学校</label><input id="ms-edu${index}-school"></div>
    <div><label for="ms-edu${index}-degree">学历</label><select id="ms-edu${index}-degree"><option value="">请选择</option><option>本科</option><option>硕士</option><option>博士</option></select></div>
    <div><label for="ms-edu${index}-major">专业</label><input id="ms-edu${index}-major"></div>
    <div><label for="ms-edu${index}-start">入学时间</label><input id="ms-edu${index}-start" type="month"></div>
    <div><label for="ms-edu${index}-end">毕业时间</label><input id="ms-edu${index}-end" type="month"></div>
    <div><label for="ms-edu${index}-gpa">GPA / 绩点</label><input id="ms-edu${index}-gpa"></div>
  </div></fieldset>`;
}

function experienceMarkup(index) {
  return `<fieldset class="repeat-card"><legend>实习经历 ${index}</legend><div class="grid">
    <div><label for="ms-exp${index}-company">公司</label><input id="ms-exp${index}-company"></div>
    <div><label for="ms-exp${index}-title">职位</label><input id="ms-exp${index}-title"></div>
    <div><label for="ms-exp${index}-location">地点</label><input id="ms-exp${index}-location"></div>
    <div><label for="ms-exp${index}-start">开始时间</label><input id="ms-exp${index}-start" type="month"></div>
    <div><label for="ms-exp${index}-end">结束时间</label><input id="ms-exp${index}-end" type="month"></div>
    <div class="full"><label for="ms-exp${index}-summary">工作内容</label><textarea id="ms-exp${index}-summary"></textarea></div>
  </div></fieldset>`;
}

function projectMarkup(index) {
  return `<fieldset class="repeat-card"><legend>项目经历 ${index}</legend><div class="grid">
    <div><label for="ms-project${index}-name">项目名称</label><input id="ms-project${index}-name"></div>
    <div><label for="ms-project${index}-role">项目角色</label><input id="ms-project${index}-role"></div>
    <div><label for="ms-project${index}-start">开始时间</label><input id="ms-project${index}-start" type="month"></div>
    <div><label for="ms-project${index}-end">结束时间</label><input id="ms-project${index}-end" type="month"></div>
    <div class="full"><label for="ms-project${index}-tech">技术栈</label><input id="ms-project${index}-tech"></div>
    <div class="full"><label for="ms-project${index}-summary">项目描述</label><textarea id="ms-project${index}-summary"></textarea></div>
    <div class="full"><label for="ms-project${index}-url">项目链接</label><input id="ms-project${index}-url" type="url"></div>
  </div></fieldset>`;
}

function add(kind) {
  state[kind] += 1;
  const index = state[kind];
  const target =
    kind === "education"
      ? document.querySelector("#ms-education")
      : kind === "experience"
        ? document.querySelector("#ms-experience")
        : document.querySelector("#ms-projects");
  const html =
    kind === "education"
      ? educationMarkup(index)
      : kind === "experience"
        ? experienceMarkup(index)
        : projectMarkup(index);
  target.insertAdjacentHTML("beforeend", html);
}

function stepTitle(step) {
  return ["基本信息", "教育经历", "实习与项目经历", "确认信息"][step - 1];
}

function renderStep() {
  document.querySelectorAll(".wizard-step").forEach((section) => {
    section.hidden = Number(section.dataset.step) !== state.step;
  });
  progress.textContent = `第 ${state.step} 步 / 共 ${totalSteps} 步 · ${stepTitle(state.step)}`;
}

document.querySelector("#ms-add-education").addEventListener("click", () => add("education"));
document.querySelector("#ms-add-experience").addEventListener("click", () => add("experience"));
document.querySelector("#ms-add-project").addEventListener("click", () => add("projects"));
document.querySelectorAll("[data-next]").forEach((button) => {
  button.addEventListener("click", () => {
    if (state.step < totalSteps) state.step += 1;
    renderStep();
  });
});

add("education");
add("experience");
add("projects");
renderStep();
