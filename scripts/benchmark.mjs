import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { spawn } from "node:child_process";
import { resolve } from "node:path";

const themes = [
  ["雷达点云", "Radar point cloud", ["目标检测算法", "传感器融合", "target tracking", "point-cloud registration"], ["arxiv.org", "github.com", "radar-course.example"]],
  ["数值流体力学", "Computational fluid dynamics", ["流场模拟", "湍流模型", "fluid solver", "mesh convergence"], ["ieeexplore.ieee.org", "github.com", "fluid-course.example"]],
  ["实习招聘", "Internship hiring", ["校招简历", "面试流程", "internship interview", "graduate recruiting"], ["www.zhipin.com", "jobs.51job.com", "career-guide.example"]],
  ["网页无障碍", "Web accessibility", ["屏幕阅读器", "键盘交互", "screen reader", "color contrast"], ["github.com", "developer.mozilla.org", "design-tools.example"]],
  ["咖啡冲泡", "Coffee brewing", ["手冲咖啡豆", "磨豆机", "pour over coffee", "coffee grinder"], ["kitchen.example", "shopping.example", "recipes.example"]],
  ["日语学习", "Japanese language", ["语法课程", "假名词汇", "grammar lessons", "kanji vocabulary"], ["language-school.example", "dictionary.example", "www.youtube.com"]],
  ["税务申报", "Tax filing", ["个人所得税", "年度汇算", "income tax forms", "tax deductions"], ["finance.example", "www.chinatax.gov.cn", "government.example"]],
];

const directory = resolve(".tools", "benchmark");
await mkdir(directory, { recursive: true });
const exe = process.argv[2] ?? resolve("dist", "TabSortLite-Edge-1.0.0", "TabSortLite.exe");

async function classify(label, tabs, windows) {
  const input = resolve(directory, `${label}.json`);
  const report = resolve(directory, `${label}-${windows}-windows.json`);
  await writeFile(input, JSON.stringify({ profileId: "synthetic", sessionId: "synthetic", revision: "synthetic", windows: [{ id: 1, focused: true }], tabs, groups: [] }));
  const child = spawn(exe, ["--benchmark", input, report, String(windows)], { windowsHide: true });
  const exit = await new Promise((done, fail) => { child.on("error", fail); child.on("exit", done); });
  if (exit !== 0) throw new Error(await readFile(report, "utf8"));
  return JSON.parse(await readFile(report, "utf8"));
}

function tab(index, title, url) {
  return { id: index + 1, windowId: 1, index, title, url, pinned: false, active: index === 0, groupId: -1, protection: null };
}

const tabs = Array.from({ length: 274 }, (_, index) => {
  const id = index + 1;
  const [zh, en, subjects, hosts] = themes[index % themes.length];
  const serial = Math.floor(index / themes.length);
  const subject = serial % subjects.length;
  return tab(index, `${subject < 2 ? zh : en} ${subjects[subject]} ${serial + 1}`, `https://${hosts[serial % hosts.length]}/topics/${id}`);
});
const result = await classify("synthetic-274", tabs, 2);
const predicted = new Map();
result.plan.windows.flatMap((window) => window.groups).forEach(({ tabIds }, cluster) => tabIds.forEach((id) => predicted.set(id, cluster)));
let tp = 0;
let fp = 0;
let fn = 0;
for (let left = 1; left <= tabs.length; left++) {
  for (let right = left + 1; right <= tabs.length; right++) {
    const sameTheme = left % themes.length === right % themes.length;
    const sameCluster = predicted.get(left) === predicted.get(right);
    if (sameTheme && sameCluster) tp++;
    if (!sameTheme && sameCluster) fp++;
    if (sameTheme && !sameCluster) fn++;
  }
}
console.log(JSON.stringify({ tabs: result.tabs, coldMs: result.coldMs, warmMs: result.warmMs, pairwiseF1: (2 * tp) / (2 * tp + fp + fn), topics: result.plan.windows.flatMap((window) => window.groups).map(({ title, tabIds }) => ({ title, tabs: tabIds.length })) }, null, 2));

const useCases = [
  ["科研与分析", /文献阅读|数据与实验/, [
    ["激光雷达点云 目标检测论文 - GitHub", "https://github.com/lab/point-cloud/papers"],
    ["LiDAR point cloud object detection preprint", "https://arxiv.org/abs/2410.12345"],
    ["激光雷达点云 实验结果", "https://research.example/experiments/point-cloud"],
    ["激光雷达点云 多传感器融合数据集", "https://datasets.example/lidar/point-cloud"],
  ]],
  ["科研与分析", /文献阅读|数据与实验/, [
    ["气候模型 极端降水 数据集 - GitHub", "https://github.com/lab/climate/datasets"],
    ["Climate modeling rainfall simulation dataset", "https://university.example/data/rainfall"],
    ["气候模型 极端降水研究论文", "https://arxiv.org/abs/2420.12345"],
    ["气候模型 降水预测实验结果", "https://research.example/experiments/rainfall"],
  ]],
  ["软件开发", /部署与运维/, [
    ["容器平台 容器镜像发布流水线 - GitHub", "https://github.com/team/containers/actions"],
    ["容器平台 容器镜像部署", "https://cloud.example/containers/deployments"],
    ["Container platform deployment pipeline", "https://dev.example/containers/pipelines"],
    ["容器平台 生产部署", "https://ops.example/containers/deployments"],
  ]],
  ["软件开发", /技术文档/, [
    ["无障碍前端 ARIA 技术文档 - GitHub", "https://github.com/team/aria/docs"],
    ["无障碍前端 ARIA 接口文档", "https://developer.mozilla.org/aria/docs"],
    ["ARIA accessibility API reference", "https://dev.example/aria/docs"],
    ["无障碍前端 键盘交互开发文档", "https://frontend.example/aria/docs"],
  ]],
  ["软件开发", /问题跟踪/, [
    ["离线同步 数据冲突错误报告 - GitHub", "https://github.com/team/offline-sync/issues/11"],
    ["离线同步 数据冲突缺陷跟踪", "https://tracker.example/offline-sync/issues/44"],
    ["Offline sync conflict bug report", "https://qa.example/offline-sync/bugs/18"],
    ["离线同步 合并请求 数据冲突", "https://gitlab.com/team/offline-sync/merge-requests/15"],
  ]],
  ["求职与职业发展", /岗位与投递/, [
    ["校招职位 数据工程师岗位投递 - GitHub", "https://github.com/community/graduate-jobs"],
    ["校招职位 数据工程师招聘", "https://employer.example/careers/data-engineer"],
    ["校招职位 数据工程师岗位列表", "https://university.example/jobs/data-engineer"],
    ["Graduate hiring data engineer open positions", "https://jobs.example/positions/data-engineer"],
  ]],
  ["求职与职业发展", /简历与面试/, [
    ["面试准备 行为面试题库 - GitHub", "https://github.com/community/interview-guide"],
    ["面试准备 行为面试模拟", "https://career.example/interviews/behavioral"],
    ["Behavioral interview preparation practice", "https://practice.example/interviews/behavioral"],
    ["面试准备 行为面试经验", "https://career.example/interviews/examples"],
  ]],
];

const rows = useCases.flatMap(([, , entries]) => entries);
const workflowTabs = rows.map(([title, url], index) => tab(index, title, url));
for (const count of [2, 3, 4]) {
  const { plan } = await classify("cross-site-workflows", workflowTabs, count);
  const groups = plan.windows.flatMap((window) => window.groups);
  console.log(JSON.stringify(plan.windows.map(({ title, groups }) => ({ title, groups: groups.map((group) => group.title) }))));
  assert.equal(plan.windows.length, count);
  assert.equal(groups.length, useCases.length);
  assert(plan.windows.every(({ groups }) => groups.length >= (count <= 3 ? 2 : 1)));
  for (const [index, [workflow, label]] of useCases.entries()) {
    const ids = [1, 2, 3, 4].map((offset) => index * 4 + offset);
    const window = plan.windows.find(({ groups }) => groups.some(({ tabIds }) => tabIds.includes(ids[0])));
    const group = window?.groups.find(({ tabIds }) => tabIds.includes(ids[0]));
    assert.deepEqual([...group.tabIds].sort((a, b) => a - b), ids);
    assert.match(group.title, label);
    if (count === 3) assert.equal(window.title, workflow);
  }
}
