const anchorUrl = chrome.runtime.getURL("anchor.html");
const editableHosts = new Set(["docs.google.com", "www.figma.com", "figma.com", "www.notion.so", "notion.so", "office.live.com", "www.office.com", "www.microsoft365.com"]);

function protectionFor(tab, groups) {
  if (tab.pinned) return "固定标签页";
  if (tab.audible) return "正在播放声音";
  if (groups.get(tab.groupId)?.shared) return "共享分组";
  if (tab.pendingUrl && tab.pendingUrl !== tab.url) return "正在导航";
  if (!isWebUrl(tab.url)) return "浏览器内部页、文件或扩展页";
  const { hostname } = new URL(tab.url);
  if (editableHosts.has(hostname) || hostname.endsWith(".sharepoint.com")) return "在线编辑页面";
  return null;
}

function isWebUrl(url) {
  return typeof url === "string" && /^https?:\/\//i.test(url) && URL.canParse(url);
}

export async function collect(identity) {
  const windows = (await chrome.windows.getAll({ populate: true, windowTypes: ["normal"] }))
    .filter((window) => !window.incognito).sort((a, b) => a.id - b.id);
  const windowIds = new Set(windows.map((window) => window.id));
  const groups = (await chrome.tabGroups.query({}))
    .filter((group) => windowIds.has(group.windowId)).sort((a, b) => a.id - b.id)
    .map(({ id, windowId, title = "", color, collapsed, shared = false }) => ({ id, windowId, title, color, collapsed, shared }));
  const byGroup = new Map(groups.map((group) => [group.id, group]));
  const tabs = windows.flatMap((window) => window.tabs.sort((a, b) => a.index - b.index).map((tab) => ({
    id: tab.id,
    windowId: tab.windowId,
    index: tab.index,
    title: tab.title ?? "",
    url: tab.url ?? "",
    pinned: tab.pinned,
    active: tab.active,
    groupId: tab.groupId,
    protection: protectionFor(tab, byGroup),
  })));
  const blockedGroups = new Set(tabs.filter((tab) => tab.groupId >= 0 && tab.protection).map((tab) => tab.groupId));
  for (const tab of tabs) {
    if (!tab.protection && blockedGroups.has(tab.groupId)) tab.protection = "同组含受保护标签页";
  }
  const structure = JSON.stringify([identity.sessionId, groups, tabs.map(({ id, windowId, index, url, groupId, protection }) => [id, windowId, index, url, groupId, protection])]);
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(structure));
  const revision = [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
  return { ...identity, revision, windows: windows.map(({ id, focused }) => ({ id, focused })), tabs, groups };
}

function requireThat(ok, message) {
  if (!ok) throw new Error(message);
}

function idsEqual(left, right) {
  return left.length === right.length && left.every((value, index) => value === right[index]);
}

function groupMembers(tabs, id) {
  return tabs.filter((tab) => tab.groupId === id).sort((a, b) => a.index - b.index).map((tab) => tab.id);
}

function validatePlan(snapshot, plan) {
  requireThat(snapshot.revision === plan.revision, "浏览器布局已变化。请重新扫描并预览。");
  requireThat(plan.windows.length >= 1 && plan.windows.length <= 6, "目标窗口数量无效");
  const tabs = new Map(snapshot.tabs.map((tab) => [tab.id, tab]));
  const approved = plan.splitGroupIds ?? [];
  requireThat(Array.isArray(approved), "原组拆分授权无效");
  const split = new Set(approved);
  requireThat(split.size === approved.length, "原组拆分授权重复");
  for (const id of split) {
    const group = snapshot.groups.find((group) => group.id === id);
    const members = snapshot.tabs.filter((tab) => tab.groupId === id);
    requireThat(group && !group.shared && members.length && members.every((tab) => !tab.protection), "所选原组已变化或含有受保护标签");
  }
  const seen = new Set();
  for (const target of plan.windows) {
    requireThat(target.groups.length > 0, "目标窗口为空");
    for (const group of target.groups) {
      requireThat(group.tabIds.length > 0, "计划包含空分组");
      for (const id of group.tabIds) {
        const tab = tabs.get(id);
        requireThat(tab && !tab.protection && !seen.has(id), "计划包含受保护、重复或缺失的标签页");
        const allowed = group.existingGroupId === null
          ? tab.groupId === -1 || split.has(tab.groupId)
          : tab.groupId === group.existingGroupId && !split.has(tab.groupId);
        requireThat(allowed, "计划会拆开未经授权的原组");
        seen.add(id);
      }
      if (group.existingGroupId !== null) {
        requireThat(idsEqual(group.tabIds, groupMembers(snapshot.tabs, group.existingGroupId)), "手动分组成员或顺序已变化");
      }
    }
  }
  requireThat(snapshot.tabs.every((tab) => tab.protection || seen.has(tab.id)), "计划遗漏可移动标签页");
  return split;
}

async function createWindow(anchors) {
  const created = await chrome.windows.create({ url: anchorUrl, focused: false, type: "normal" });
  const anchor = created.tabs?.[0] ?? (await chrome.tabs.query({ windowId: created.id }))[0];
  requireThat(anchor, "无法获取临时窗口");
  anchors.push(anchor.id);
  return created.id;
}

async function clearAnchors(anchors) {
  for (const id of anchors) {
    const tab = await chrome.tabs.get(id).catch(() => null);
    if (tab && !tab.pinned && tab.groupId === -1 && tab.url === anchorUrl && (!tab.pendingUrl || tab.pendingUrl === anchorUrl)) {
      await chrome.tabs.remove(id);
    }
  }
}

export async function execute(identity, plan) {
  const before = await collect(identity);
  const split = validatePlan(before, plan);
  const anchors = [];
  const expected = [];
  const movedIds = new Set();
  const originalById = new Map(before.tabs.map((tab) => [tab.id, tab]));
  try {
    for (const target of plan.windows) {
      const windowId = await createWindow(anchors);
      for (const group of target.groups) {
        const live = await collect(identity);
        const byId = new Map(live.tabs.map((tab) => [tab.id, tab]));
        requireThat(group.tabIds.every((id) => {
          const tab = byId.get(id);
          const original = originalById.get(id);
          return tab && !tab.protection && tab.url === original.url && tab.groupId === original.groupId && tab.windowId === original.windowId;
        }), "待移动的标签页已变化或受到保护");
        let groupId;
        if (group.existingGroupId !== null) {
          requireThat(idsEqual(group.tabIds, groupMembers(live.tabs, group.existingGroupId)), "手动分组在整理期间已变化");
          const originalGroup = before.groups.find(({ id }) => id === group.existingGroupId);
          const liveGroup = live.groups.find(({ id }) => id === group.existingGroupId);
          requireThat(liveGroup && liveGroup.title === originalGroup.title && liveGroup.color === originalGroup.color, "手动分组名称或颜色已变化");
          const moved = await chrome.tabGroups.move(group.existingGroupId, { windowId, index: -1 });
          groupId = moved?.id ?? (await chrome.tabs.get(group.tabIds[0])).groupId;
        } else {
          const grouped = group.tabIds.filter((id) => split.has(originalById.get(id).groupId));
          const sources = new Set(grouped.map((id) => originalById.get(id).groupId));
          for (const source of sources) {
            const original = before.groups.find(({ id }) => id === source);
            const current = live.groups.find(({ id }) => id === source);
            const remaining = groupMembers(before.tabs, source).filter((id) => !movedIds.has(id));
            requireThat(current?.title === original.title && current.color === original.color &&
              idsEqual(remaining, groupMembers(live.tabs, source)), "所选原组在整理期间已变化");
          }
          if (grouped.length) await chrome.tabs.ungroup(grouped);
          await chrome.tabs.move(group.tabIds, { windowId, index: -1 });
          groupId = await chrome.tabs.group({ tabIds: group.tabIds, createProperties: { windowId } });
          await chrome.tabGroups.update(groupId, { title: group.title, color: group.color });
        }
        for (const id of group.tabIds) movedIds.add(id);
        expected.push({ groupId, windowId, group });
      }
    }
  } finally {
    await clearAnchors(anchors);
  }
  const after = await collect(identity);
  const afterIds = new Set(after.tabs.map((tab) => tab.id));
  requireThat(before.tabs.every((tab) => afterIds.has(tab.id)), "校验发现有原始标签页缺失，请检查并恢复布局");
  for (const { groupId, windowId, group } of expected) {
    const actual = after.groups.find(({ id }) => id === groupId);
    requireThat(actual?.windowId === windowId && actual.title === group.title && actual.color === group.color && idsEqual(group.tabIds, groupMembers(after.tabs, groupId)), "整理后的分组校验失败，请检查并恢复布局");
  }
  return { snapshot: after, moved: expected.reduce((sum, entry) => sum + entry.group.tabIds.length, 0) };
}

export async function restore(identity, saved) {
  requireThat(saved.profileId === identity.profileId, "此快照属于另一个 Edge 配置文件");
  const live = await collect(identity);
  const sameSession = saved.sessionId === live.sessionId;
  const journalKey = `restore-${saved.revision}`;
  const { [journalKey]: journal = { tabs: {}, windows: {} } } = await chrome.storage.session.get(journalKey);
  const checkpoint = () => chrome.storage.session.set({ [journalKey]: journal });
  const remaining = new Map(live.tabs.map((tab) => [tab.id, tab]));
  const windows = new Set(live.windows.map((window) => window.id));
  const originalIds = new Set([...Object.values(journal.tabs), ...(sameSession ? saved.tabs.map((tab) => tab.id) : [])]);
  const protectedGroups = new Set(live.tabs.filter((tab) => tab.groupId >= 0 && (tab.protection || !originalIds.has(tab.id))).map((tab) => tab.groupId));
  const savedGroups = new Map(saved.groups.map((group) => [group.id, group]));
  const anchors = [];
  const restored = new Map();
  let reopened = 0;
  let skipped = 0;
  try {
    for (const original of saved.windows) {
      const tabs = saved.tabs.filter((tab) => tab.windowId === original.id).sort((a, b) => a.index - b.index);
      const trackedWindow = journal.windows[original.id];
      let windowId = sameSession && windows.has(original.id) ? original.id : windows.has(trackedWindow) ? trackedWindow : null;
      const handledGroups = new Set();
      for (const tab of tabs) {
        const current = remaining.get(journal.tabs[tab.id] ?? (sameSession ? tab.id : undefined));
        if (tab.protection || current?.protection || protectedGroups.has(current?.groupId)) {
          if (!(current?.windowId === windowId && current.url === tab.url)) skipped++;
          continue;
        }
        if (current?.url !== tab.url && !isWebUrl(tab.url)) {
          skipped++;
          continue;
        }
        if (tab.groupId >= 0 && handledGroups.has(tab.groupId)) continue;
        if (windowId === null) {
          windowId = await createWindow(anchors);
          journal.windows[original.id] = windowId;
          await checkpoint();
        }

        // Restore an intact original manual group with the same native move API.
        const members = tab.groupId >= 0 ? groupMembers(tabs, tab.groupId) : [];
        if (sameSession && current?.id === tab.id && current.groupId >= 0 && members.length &&
            idsEqual(members, groupMembers(live.tabs, current.groupId)) &&
            members.every((id) => remaining.get(id).url === tabs.find((item) => item.id === id).url)) {
          const groupId = (await chrome.tabGroups.move(current.groupId, { windowId, index: -1 }))?.id ?? (await chrome.tabs.get(tab.id)).groupId;
          const { title, color, collapsed } = savedGroups.get(tab.groupId);
          await chrome.tabGroups.update(groupId, { title, color, collapsed });
          for (const id of members) restored.set(id, id);
          handledGroups.add(tab.groupId);
          continue;
        }

        if (current?.url === tab.url) {
          if (current.groupId >= 0) await chrome.tabs.ungroup(current.id);
          await chrome.tabs.move(current.id, { windowId, index: -1 });
          restored.set(tab.id, current.id);
        } else {
          const created = await chrome.tabs.create({ windowId, url: tab.url, active: false });
          restored.set(tab.id, created.id);
          journal.tabs[tab.id] = created.id;
          await checkpoint();
          reopened++;
        }
      }

      if (windowId === null) continue;
      for (const tab of tabs) {
        const id = restored.get(tab.id);
        if (id === undefined || handledGroups.has(tab.groupId)) continue;
        if (tab.groupId < 0) {
          await chrome.tabs.move(id, { windowId, index: tab.index });
        } else {
          const members = groupMembers(tabs, tab.groupId);
          if (!members.every((member) => restored.has(member))) continue;
          const tabIds = members.map((member) => restored.get(member));
          const groupId = await chrome.tabs.group({ tabIds, createProperties: { windowId } });
          const { title, color, collapsed } = savedGroups.get(tab.groupId);
          await chrome.tabGroups.update(groupId, { title, color, collapsed });
          await chrome.tabGroups.move(groupId, { windowId, index: tab.index });
          handledGroups.add(tab.groupId);
        }
      }
      for (const groupId of handledGroups) {
        const first = tabs.find((tab) => tab.groupId === groupId);
        const actual = await chrome.tabs.get(restored.get(first.id));
        await chrome.tabGroups.move(actual.groupId, { windowId, index: first.index });
      }
      const active = tabs.find((tab) => tab.active);
      if (active && restored.has(active.id)) await chrome.tabs.update(restored.get(active.id), { active: true });
    }
  } finally {
    await clearAnchors(anchors);
  }
  return { snapshot: await collect(identity), restored: restored.size, reopened, skipped };
}
