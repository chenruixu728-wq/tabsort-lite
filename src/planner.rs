use crate::intent::{self, Page};
use crate::protocol::{Plan, Snapshot, Tab, TargetGroup, TargetWindow};
use crate::semantic::{dot, normalize, Model};
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Detail {
    Coarse,
    Standard,
    Fine,
}

impl Detail {
    pub const OPTIONS: [(Self, &'static str); 3] = [
        (Self::Coarse, "粗分类 · 约 3 个主题"),
        (Self::Standard, "标准 · 约 7 个主题"),
        (Self::Fine, "精细 · 约 12 个主题"),
    ];

    fn groups(self) -> usize {
        match self {
            Self::Coarse => 3,
            Self::Standard => 7,
            Self::Fine => 12,
        }
    }
}

pub const COLORS: &[&str] = &[
    "blue", "green", "purple", "orange", "cyan", "pink", "yellow", "red", "grey",
];

pub fn duplicate_count(snapshot: &Snapshot) -> usize {
    let mut seen_urls = BTreeSet::new();
    snapshot
        .tabs
        .iter()
        .filter(|tab| !tab.url.is_empty() && !seen_urls.insert(&tab.url))
        .count()
}

pub fn preview(
    snapshot: &Snapshot,
    model: &mut Model,
    requested: usize,
    detail: Detail,
) -> Result<Plan> {
    let tabs: Vec<_> = snapshot
        .tabs
        .iter()
        .filter(|tab| tab.protection.is_none())
        .collect();
    let pages: Vec<_> = tabs.iter().map(|tab| intent::analyze(tab)).collect();
    let texts: Vec<_> = pages.iter().map(|page| page.description.clone()).collect();
    let vectors = model.embed_topics(&texts)?;
    Ok(plan(snapshot, tabs, &pages, &vectors, requested, detail))
}

fn plan(
    snapshot: &Snapshot,
    tabs: Vec<&Tab>,
    pages: &[Page],
    vectors: &[Vec<f32>],
    requested: usize,
    detail: Detail,
) -> Plan {
    let mut manual = BTreeMap::<i32, Vec<usize>>::new();
    let mut ungrouped = Vec::new();
    for (index, tab) in tabs.iter().enumerate() {
        if tab.group_id < 0 {
            ungrouped.push(index);
        } else {
            manual.entry(tab.group_id).or_default().push(index);
        }
    }

    let mut result_groups = Vec::new();
    let mut group_vectors = Vec::new();
    let mut workflows = Vec::new();
    for group in &snapshot.groups {
        if let Some(members) = manual.get_mut(&group.id) {
            members.sort_by_key(|&index| tabs[index].index);
            result_groups.push(TargetGroup {
                title: group.title.clone(),
                color: group.color.clone(),
                existing_group_id: Some(group.id),
                tab_ids: members.iter().map(|&index| tabs[index].id).collect(),
            });
            group_vectors.push(centroid(vectors, members));
            workflows.push(workflow_evidence(pages, members));
        }
    }

    let target = detail
        .groups()
        .saturating_sub(result_groups.len())
        .max(1)
        .min(ungrouped.len().div_ceil(2));
    let term_lists: Vec<_> = ungrouped
        .iter()
        .map(|&index| terms(&pages[index].subject))
        .collect();
    let mut frequencies = HashMap::new();
    for term in term_lists.iter().flatten() {
        *frequencies.entry(term.as_str()).or_insert(0) += 1;
    }
    let scores = pair_scores(ungrouped.len(), |left, right| {
        let (a, b) = (ungrouped[left], ungrouped[right]);
        dot(&vectors[a], &vectors[b])
    });
    let mut used_names = result_groups
        .iter()
        .map(|group| group.title.clone())
        .collect::<BTreeSet<_>>();
    for cluster in merge(scores, ungrouped.len(), target) {
        let members: Vec<_> = cluster.iter().map(|&index| ungrouped[index]).collect();
        let center = centroid(vectors, &members);
        let representative = members
            .iter()
            .copied()
            .filter(|&index| !pages[index].subject.is_empty())
            .max_by(|&left, &right| {
                dot(&vectors[left], &center).total_cmp(&dot(&vectors[right], &center))
            });
        let name = theme_name(
            &cluster,
            &term_lists,
            &frequencies,
            pages,
            &members,
            representative,
        );
        let label = unique_name(name, &mut used_names);
        let color = COLORS[result_groups.len() % COLORS.len()];
        result_groups.push(TargetGroup {
            title: label,
            color: color.to_owned(),
            existing_group_id: None,
            tab_ids: members.iter().map(|&index| tabs[index].id).collect(),
        });
        group_vectors.push(center);
        workflows.push(workflow_evidence(pages, &members));
    }

    let group_count = result_groups.len();
    let scores = pair_scores(group_count, |left, right| {
        let common_goal = workflows[left]
            .iter()
            .filter_map(|(name, a)| workflows[right].get(name).map(|b| a.min(*b)))
            .sum::<f32>();
        dot(&group_vectors[left], &group_vectors[right]) + 0.18 * common_goal
    });
    let mut windows = merge(scores.clone(), group_count, requested);
    spread_groups(&mut windows, &scores, group_count);
    let titles: Vec<_> = windows
        .iter()
        .map(|members| window_name(members, &workflows, &result_groups, &scores))
        .collect();
    let mut window_names = BTreeSet::new();
    let windows = windows
        .into_iter()
        .zip(&titles)
        .map(|(members, base)| {
            let title = if titles.iter().filter(|name| *name == base).count() > 1 {
                let topics = central_subjects(&members, &result_groups, &scores, 1);
                topics
                    .first()
                    .map_or_else(|| base.clone(), |topic| format!("{base} · {topic}"))
            } else {
                base.clone()
            };
            let title = unique_name(title, &mut window_names);
            TargetWindow {
                title,
                groups: members
                    .into_iter()
                    .map(|index| result_groups[index].clone())
                    .collect(),
            }
        })
        .collect();
    Plan {
        revision: snapshot.revision.clone(),
        windows,
        protected_count: snapshot.tabs.len() - tabs.len(),
        duplicate_count: duplicate_count(snapshot),
        split_group_ids: Vec::new(),
    }
}

fn centroid(vectors: &[Vec<f32>], members: &[usize]) -> Vec<f32> {
    let mut center = vec![0.0; vectors[members[0]].len()];
    for &index in members {
        for (sum, value) in center.iter_mut().zip(&vectors[index]) {
            *sum += value;
        }
    }
    normalize(&mut center);
    center
}

pub fn pair_scores(size: usize, score: impl Fn(usize, usize) -> f32) -> Vec<f32> {
    let mut scores = vec![0.0; size * size];
    for left in 0..size {
        for right in left + 1..size {
            let similarity = score(left, right);
            scores[left * size + right] = similarity;
            scores[right * size + left] = similarity;
        }
    }
    scores
}

// Average-linkage clustering updates a fixed similarity table after each merge.
fn merge(mut similarities: Vec<f32>, size: usize, count: usize) -> Vec<Vec<usize>> {
    let mut members: Vec<_> = (0..size).map(|index| vec![index]).collect();
    if size <= count.max(1) {
        return members;
    }
    let mut active: Vec<_> = (0..size).collect();
    while active.len() > count.max(1) {
        let mut best = (f32::NEG_INFINITY, 0, 1);
        for (position, &left) in active.iter().enumerate() {
            for (offset, &right) in active[position + 1..].iter().enumerate() {
                let score = similarities[left * size + right];
                if score > best.0 {
                    best = (score, position, position + 1 + offset);
                }
            }
        }
        let left = active[best.1];
        let right = active.remove(best.2);
        let left_size = members[left].len() as f32;
        let right_size = members[right].len() as f32;
        for &other in &active {
            let score = (similarities[left * size + other] * left_size
                + similarities[right * size + other] * right_size)
                / (left_size + right_size);
            similarities[left * size + other] = score;
            similarities[other * size + left] = score;
        }
        let absorbed = std::mem::take(&mut members[right]);
        members[left].extend(absorbed);
    }
    active
        .into_iter()
        .map(|index| {
            members[index].sort_unstable();
            std::mem::take(&mut members[index])
        })
        .collect()
}

fn affinity(group: usize, window: &[usize], scores: &[f32], size: usize) -> f32 {
    let total: f32 = window
        .iter()
        .filter(|&&other| other != group)
        .map(|&other| scores[group * size + other])
        .sum();
    let peers = window.len() - usize::from(window.contains(&group));
    total / peers.max(1) as f32
}

// Bound the number of groups per window. A large project may still own most tabs.
pub fn spread_groups(windows: &mut [Vec<usize>], scores: &[f32], total: usize) {
    if windows.len() <= 1 {
        return;
    }
    let (min, max) = window_group_bounds(total, windows.len());
    while windows
        .iter()
        .any(|window| window.len() < min || window.len() > max)
    {
        let mut best: Option<(f32, usize, usize, usize)> = None;
        for (from, source) in windows
            .iter()
            .enumerate()
            .filter(|(_, source)| source.len() > min)
        {
            for (to, target) in windows
                .iter()
                .enumerate()
                .filter(|(to, target)| *to != from && target.len() < max)
            {
                if source.len() <= max && target.len() >= min {
                    continue;
                }
                for &group in source {
                    let gain = affinity(group, target, scores, total)
                        - affinity(group, source, scores, total);
                    if best.is_none_or(|(best_gain, ..)| gain > best_gain) {
                        best = Some((gain, from, to, group));
                    }
                }
            }
        }
        let Some((_, from, to, group)) = best else {
            break;
        };
        windows[from].retain(|&member| member != group);
        windows[to].push(group);
    }
    for window in windows {
        window.sort_unstable();
    }
}

pub fn window_group_bounds(groups: usize, windows: usize) -> (usize, usize) {
    let average = groups / windows;
    let min = if average >= 2 {
        (average - 1).max(2)
    } else {
        1
    };
    let max = groups.div_ceil(windows) + usize::from(average >= 2);
    (min, max)
}

fn workflow_evidence(pages: &[Page], members: &[usize]) -> BTreeMap<&'static str, f32> {
    let mut evidence = BTreeMap::new();
    for purpose in members.iter().filter_map(|&index| pages[index].purpose) {
        *evidence.entry(purpose.workflow).or_insert(0.0) += 1.0 / members.len() as f32;
    }
    evidence
}

fn window_name(
    members: &[usize],
    workflows: &[BTreeMap<&str, f32>],
    groups: &[TargetGroup],
    scores: &[f32],
) -> String {
    let mut evidence = BTreeMap::new();
    for &index in members {
        for (&workflow, &weight) in &workflows[index] {
            *evidence.entry(workflow).or_insert(0.0_f32) += weight;
        }
    }
    let mut evidence: Vec<_> = evidence.into_iter().collect();
    evidence.sort_by(|a, b| b.1.total_cmp(&a.1));
    if let Some(&(goal, weight)) = evidence
        .first()
        .filter(|(_, weight)| *weight >= 0.5 && *weight * 4.0 >= members.len() as f32)
    {
        return match evidence.get(1) {
            Some(&(second, other)) if other >= 0.75 && other >= weight * 0.5 => {
                format!("{goal} / {second}")
            }
            _ => goal.to_owned(),
        };
    }
    let labels = central_subjects(members, groups, scores, 2);
    if labels.is_empty() {
        "待识别场景".into()
    } else {
        labels.join(" / ")
    }
}

fn central_subjects<'a>(
    members: &[usize],
    groups: &'a [TargetGroup],
    scores: &[f32],
    limit: usize,
) -> Vec<&'a str> {
    let mut topics = members.to_vec();
    topics.sort_by(|&a, &b| {
        affinity(b, members, scores, groups.len()).total_cmp(&affinity(
            a,
            members,
            scores,
            groups.len(),
        ))
    });
    topics
        .into_iter()
        .map(|index| groups[index].title.split(" · ").next().unwrap_or_default())
        .filter(|title| !title.is_empty() && *title != "待识别用途")
        .take(limit)
        .collect()
}

fn terms(value: &str) -> BTreeSet<String> {
    let mut words = BTreeSet::new();
    for token in value.split(|ch: char| !ch.is_alphanumeric()) {
        if token.is_ascii() {
            let token = token.to_lowercase();
            if token.len() >= 3
                && token.chars().any(|ch| ch.is_ascii_alphabetic())
                && ![
                    "www", "com", "the", "for", "and", "with", "github", "search", "page",
                ]
                .contains(&token.as_str())
            {
                words.insert(token);
            }
        } else {
            let chars: Vec<_> = token.chars().take(35).collect();
            for length in 2..=7 {
                for part in chars.windows(length) {
                    words.insert(part.iter().collect());
                }
            }
        }
    }
    words
}

fn theme_name(
    cluster: &[usize],
    terms: &[BTreeSet<String>],
    totals: &HashMap<&str, usize>,
    pages: &[Page],
    members: &[usize],
    representative: Option<usize>,
) -> String {
    let mut counts = BTreeMap::new();
    for &index in cluster {
        for term in &terms[index] {
            *counts.entry(term.as_str()).or_insert(0) += 1;
        }
    }
    let weight = |term: &str, count: usize| {
        let outside = totals[term] - count;
        count * count * term.chars().count().min(6) / (1 + outside)
    };
    let common_subject = counts
        .into_iter()
        .filter(|(_, count)| *count >= 2 && *count * 2 >= cluster.len())
        .filter(|(term, _)| {
            !members
                .iter()
                .filter_map(|&index| pages[index].purpose)
                .any(|purpose| purpose.is_action_word(term))
        })
        .max_by_key(|(term, count)| weight(term, *count))
        .map(|(term, _)| term);
    let mut actions = BTreeMap::new();
    for purpose in members.iter().filter_map(|&index| pages[index].purpose) {
        *actions.entry(purpose.name).or_insert(0) += 1;
    }
    let mut actions: Vec<_> = actions
        .into_iter()
        .filter(|(_, count)| count * 4 >= members.len())
        .collect();
    actions.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    let purpose = actions
        .into_iter()
        .take(2)
        .map(|(name, _)| name)
        .collect::<Vec<_>>()
        .join("、");
    let fallback =
        representative.map(|index| pages[index].subject.chars().take(22).collect::<String>());
    let subject = common_subject.or(fallback.as_deref()).unwrap_or_default();
    if purpose.is_empty() {
        return if subject.is_empty() {
            "待识别用途".into()
        } else {
            subject.into()
        };
    }
    match common_subject.filter(|term| !purpose.contains(term)) {
        Some(subject) => format!("{subject} · {purpose}"),
        None => purpose,
    }
}

fn unique_name(name: String, used: &mut BTreeSet<String>) -> String {
    if used.insert(name.clone()) {
        return name;
    }
    for index in 2.. {
        let candidate = format!("{name} {index}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_spread_repairs_outliers_without_reassigning_tabs() {
        let groups = 12;
        let scores = pair_scores(groups, |a, b| {
            if b < groups - 1 {
                0.95
            } else {
                0.3 + a as f32 / 100.0
            }
        });
        for count in [2, 3, 4, 6] {
            let mut windows = merge(scores.clone(), groups, count);
            assert_eq!(windows.iter().map(Vec::len).min(), Some(1));
            spread_groups(&mut windows, &scores, groups);
            assert_eq!(windows.len(), count);
            assert!(windows.iter().all(|window| window.len() >= 2));
            assert!(windows
                .iter()
                .all(|window| window.len() <= groups.div_ceil(count) + 1));
            let mut assigned = windows.into_iter().flatten().collect::<Vec<_>>();
            assigned.sort_unstable();
            assert_eq!(assigned, (0..groups).collect::<Vec<_>>());
        }
    }
}
