const containers = {
  education: document.querySelector("#education-sections"),
  experience: document.querySelector("#experience-sections"),
  projects: document.querySelector("#project-sections"),
};

const counts = { education: 0, experience: 0, projects: 0 };
const maxSections = 4;

function educationMarkup(index) {
  return `<fieldset class="repeat-card"><legend>教育经历 ${index}</legend><div class="grid">
    <div><label for="edu${index}-school">学校</label><input id="edu${index}-school"></div>
    <div><label for="edu${index}-degree">学历</label><select id="edu${index}-degree"><option value="">请选择</option><option>本科</option><option>硕士</option><option>博士</option></select></div>
    <div><label for="edu${index}-major">专业</label><input id="edu${index}-major"></div>
    <div><label for="edu${index}-start">入学时间</label><input id="edu${index}-start" type="month"></div>
    <div><label for="edu${index}-end">毕业时间</label><input id="edu${index}-end" type="month"></div>
    <div><label for="edu${index}-gpa">GPA / 绩点</label><input id="edu${index}-gpa"></div>
  </div></fieldset>`;
}

function experienceMarkup(index) {
  return `<fieldset class="repeat-card"><legend>实习经历 ${index}</legend><div class="grid">
    <div><label for="exp${index}-company">公司</label><input id="exp${index}-company"></div>
    <div><label for="exp${index}-title">职位</label><input id="exp${index}-title"></div>
    <div><label for="exp${index}-location">地点</label><input id="exp${index}-location"></div>
    <div><label for="exp${index}-start">开始时间</label><input id="exp${index}-start" type="month"></div>
    <div><label for="exp${index}-end">结束时间</label><input id="exp${index}-end" type="month"></div>
    <div class="full"><label for="exp${index}-summary">工作内容</label><textarea id="exp${index}-summary"></textarea></div>
  </div></fieldset>`;
}

function projectMarkup(index) {
  return `<fieldset class="repeat-card"><legend>项目经历 ${index}</legend><div class="grid">
    <div><label for="project${index}-name">项目名称</label><input id="project${index}-name"></div>
    <div><label for="project${index}-role">项目角色</label><input id="project${index}-role"></div>
    <div><label for="project${index}-start">开始时间</label><input id="project${index}-start" type="month"></div>
    <div><label for="project${index}-end">结束时间</label><input id="project${index}-end" type="month"></div>
    <div class="full"><label for="project${index}-tech">技术栈</label><input id="project${index}-tech"></div>
    <div class="full"><label for="project${index}-summary">项目描述</label><textarea id="project${index}-summary"></textarea></div>
    <div class="full"><label for="project${index}-url">项目链接</label><input id="project${index}-url" type="url"></div>
  </div></fieldset>`;
}

function addSection(kind) {
  if (counts[kind] >= maxSections) return;
  counts[kind] += 1;
  const index = counts[kind];
  const html =
    kind === "education"
      ? educationMarkup(index)
      : kind === "experience"
        ? experienceMarkup(index)
        : projectMarkup(index);
  containers[kind].insertAdjacentHTML("beforeend", html);
}

document.querySelector("#add-education").addEventListener("click", () => addSection("education"));
document.querySelector("#add-experience").addEventListener("click", () => addSection("experience"));
document.querySelector("#add-project").addEventListener("click", () => addSection("projects"));

addSection("education");
addSection("experience");
addSection("projects");
