use crate::bridge::{self, Browser};
use crate::planner::{self, Detail};
use crate::protocol::{Plan, Snapshot};
use crate::theme::{self, card};
use crate::{ai, recovery, semantic};
use anyhow::{bail, Context, Result};
use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, RichText};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

const ACCENT: Color32 = theme::INK;
const SUBTLE: Color32 = theme::MUTED;

pub fn run() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([720.0, 820.0])
            .with_min_inner_size([490.0, 540.0]),
        vsync: true,
        ..Default::default()
    };
    eframe::run_native(
        "TabSort Lite",
        options,
        Box::new(|context| {
            configure(&context.egui_ctx);
            Ok(Box::new(App::new()))
        }),
    )
}

fn configure(context: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let windows = std::env::var_os("WINDIR").unwrap_or_else(|| r"C:\Windows".into());
    let font_dir = std::path::PathBuf::from(windows).join("Fonts");
    if let Ok(bytes) = std::fs::read(font_dir.join("segoeui.ttf")) {
        fonts
            .font_data
            .insert("system-sans".into(), FontData::from_owned(bytes).into());
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, "system-sans".into());
    }
    for name in ["msyh.ttc", "simhei.ttf", "simsun.ttc"] {
        if let Ok(bytes) = std::fs::read(font_dir.join(name)) {
            fonts
                .font_data
                .insert("cjk".into(), FontData::from_owned(bytes).into());
            for family in [FontFamily::Proportional, FontFamily::Monospace] {
                fonts.families.entry(family).or_default().push("cjk".into());
            }
            break;
        }
    }
    context.set_fonts(fonts);
    theme::configure(context);
}

#[derive(Clone)]
struct Preview {
    browser: Browser,
    snapshot: Snapshot,
    plan: Plan,
    ai: Option<ai::AnalysisSummary>,
}

struct AiDialog {
    browser: Browser,
    split: BTreeSet<i32>,
    error: Option<String>,
}

enum Job {
    Scan(Vec<Browser>),
    Preview(Vec<Browser>, usize, Detail),
    AiPreview(Browser, ai::Settings, BTreeSet<i32>),
    Execute(Vec<Preview>),
    Restore(Vec<Browser>),
}

enum Output {
    Preview(Vec<Preview>),
    Browsers(Vec<Browser>, String),
}

struct App {
    browsers: BTreeMap<String, Browser>,
    events: Receiver<bridge::Event>,
    submit: Sender<Job>,
    outputs: Receiver<Result<Output>>,
    profile: Option<String>,
    windows: usize,
    detail: Detail,
    previews: Vec<Preview>,
    busy: bool,
    status: Option<(String, bool)>,
    restore_confirm: bool,
    ai_dialog: Option<AiDialog>,
    ai_endpoint: String,
    ai_model: String,
    ai_key: String,
    ai_running: bool,
}

impl App {
    fn new() -> Self {
        let (submit, jobs) = mpsc::channel();
        let (result, outputs) = mpsc::channel();
        std::thread::spawn(move || {
            let mut model = None;
            for job in jobs {
                if result.send(perform(job, &mut model)).is_err() {
                    break;
                }
            }
        });
        Self {
            browsers: BTreeMap::new(),
            events: bridge::listen(),
            submit,
            outputs,
            profile: None,
            windows: 2,
            detail: Detail::Standard,
            previews: Vec::new(),
            busy: false,
            status: None,
            restore_confirm: false,
            ai_dialog: None,
            ai_endpoint: ai::DEEPSEEK_ENDPOINT.into(),
            ai_model: ai::DEEPSEEK_MODEL.into(),
            ai_key: String::new(),
            ai_running: false,
        }
    }

    fn selected(&self) -> Vec<Browser> {
        self.browsers
            .values()
            .filter(|browser| {
                self.profile
                    .as_ref()
                    .is_none_or(|profile| profile == &browser.snapshot.profile_id)
            })
            .cloned()
            .collect()
    }

    fn start(&mut self, job: Job, description: &str) {
        let ai_running = matches!(&job, Job::AiPreview(..));
        if self.submit.send(job).is_ok() {
            self.busy = true;
            self.ai_running = ai_running;
            self.status = Some((description.to_owned(), false));
        } else {
            self.status = Some(("任务已中断，请重新打开 TabSort Lite。".into(), true));
        }
    }

    fn poll(&mut self) {
        for event in self.events.try_iter() {
            match event {
                bridge::Event::Connected(browser) => {
                    self.browsers
                        .insert(browser.snapshot.profile_id.clone(), browser);
                    self.previews.clear();
                }
                bridge::Event::Disconnected(connection) => {
                    self.browsers
                        .retain(|_, browser| browser.connection != connection);
                    self.previews
                        .retain(|preview| preview.browser.connection != connection);
                    if self
                        .profile
                        .as_ref()
                        .is_some_and(|profile| !self.browsers.contains_key(profile))
                    {
                        self.profile = None;
                    }
                }
                bridge::Event::ListenerError(message) => self.status = Some((message, true)),
            }
        }
        for output in self.outputs.try_iter() {
            self.busy = false;
            self.ai_running = false;
            match output {
                Ok(Output::Preview(previews)) => {
                    self.previews =
                        previews
                            .into_iter()
                            .filter(|item| {
                                self.browsers.get(&item.snapshot.profile_id).is_some_and(
                                    |browser| browser.connection == item.browser.connection,
                                )
                            })
                            .collect();
                    for item in &self.previews {
                        if let Some(browser) = self.browsers.get_mut(&item.snapshot.profile_id) {
                            browser.snapshot = item.snapshot.clone();
                        }
                    }
                    self.status = if self
                        .previews
                        .iter()
                        .any(|item| !item.plan.windows.is_empty())
                    {
                        None
                    } else {
                        Some(("没有可移动的标签页。".into(), false))
                    };
                }
                Ok(Output::Browsers(browsers, message)) => {
                    for update in browsers {
                        if let Some(browser) = self
                            .browsers
                            .get_mut(&update.snapshot.profile_id)
                            .filter(|item| item.connection == update.connection)
                        {
                            browser.snapshot = update.snapshot;
                        }
                    }
                    self.previews.clear();
                    self.status = Some((message, false));
                }
                Err(error) => {
                    self.previews.clear();
                    self.status = Some((format!("{error:#}"), true));
                }
            }
        }
    }

    fn intro(&mut self, ui: &mut egui::Ui) {
        let width = ui.available_width();
        let painter = ui.painter();
        let name = painter.layout_no_wrap(
            "TabSort Lite".into(),
            egui::FontId::proportional(23.0),
            theme::INK,
        );
        let mode = if self.ai_running || self.previews.iter().any(|preview| preview.ai.is_some()) {
            "●  AI 分析"
        } else {
            "●  本地模式"
        };
        let mode =
            painter.layout_no_wrap(mode.into(), egui::FontId::proportional(12.0), theme::INK);
        let brand_width = 34.0 + 12.0 + name.size().x;
        let badge_width = mode.size().x + 20.0;
        let copy_width = (width - brand_width - badge_width - 32.0).max(1.0);
        let heading = painter.layout(
            "让每个标签，都有位置。".into(),
            egui::FontId::proportional(18.0),
            theme::INK,
            copy_width,
        );
        let description = painter.layout(
            "扫描、理解、归类，把散落的页面收进工作场景。".into(),
            egui::FontId::proportional(12.0),
            SUBTLE,
            copy_width,
        );
        let copy_height = heading.size().y + 4.0 + description.size().y;
        let height = copy_height.max(34.0).max(name.size().y);
        // Measure the whole header before positioning its four parts on one centerline.
        let (row, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
        let painter = ui.painter();
        let center_y = row.center().y;
        let icon = egui::Rect::from_center_size(
            egui::pos2(row.left() + 17.0, center_y),
            egui::vec2(34.0, 34.0),
        );
        painter.rect_filled(icon, 11, theme::INK);
        for (index, color) in [theme::BLUE, theme::MINT, theme::LILAC]
            .into_iter()
            .enumerate()
        {
            let bar = egui::Rect::from_min_size(
                icon.min + egui::vec2(8.0, 8.0 + index as f32 * 7.0),
                egui::vec2(18.0 - index as f32 * 3.0, 4.0),
            );
            painter.rect_filled(bar, 2, color);
        }
        painter.galley(
            egui::pos2(icon.right() + 12.0, center_y - name.size().y / 2.0),
            name,
            theme::INK,
        );
        let copy_top = egui::pos2(
            row.left() + brand_width + 16.0,
            center_y - copy_height / 2.0,
        );
        let description_top = copy_top + egui::vec2(0.0, heading.size().y + 4.0);
        painter.galley(copy_top, heading, theme::INK);
        painter.galley(description_top, description, SUBTLE);
        let badge = egui::Rect::from_center_size(
            egui::pos2(row.right() - badge_width / 2.0, center_y),
            egui::vec2(badge_width, 30.0),
        );
        painter.rect_filled(badge, 10, theme::STONE);
        painter.galley(badge.center() - mode.size() / 2.0, mode, theme::INK);
        ui.add_space(6.0);
    }

    fn connection(&mut self, ui: &mut egui::Ui, browsers: &[Browser]) {
        let mut stage = egui::Frame::new()
            .fill(theme::BLUE)
            .corner_radius(24)
            .inner_margin(16)
            .begin(ui);
        let backdrop = stage.content_ui.painter().add(egui::Shape::Noop);
        card(&mut stage.content_ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("01  /  浏览器连接").size(12.0).color(SUBTLE));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    theme::badge(
                        ui,
                        if browsers.is_empty() {
                            "等待连接"
                        } else {
                            "已连接"
                        },
                        if browsers.is_empty() {
                            theme::STONE
                        } else {
                            theme::MINT
                        },
                    );
                });
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Microsoft Edge").size(19.0));
                if browsers.is_empty() {
                    ui.label(RichText::new("连接后自动扫描").color(SUBTLE));
                } else {
                    let count = browsers
                        .iter()
                        .map(|item| item.snapshot.tabs.len())
                        .sum::<usize>();
                    ui.label(
                        RichText::new(format!("{count} 个标签页"))
                            .color(ACCENT)
                            .size(19.0),
                    );
                }
            });
            if self.browsers.is_empty() {
                ui.label("首次使用请注册连接，再到 edge://extensions 加载本程序文件夹中的 extension。连接后会自动扫描，也可点 Edge 扩展图标立即连接。");
                if ui.button("注册 Edge 连接").clicked() {
                    self.status = Some(match crate::register_edge() {
                        Ok(()) => (
                            "连接已注册。请在 Edge 中加载扩展，并点击扩展图标。".into(),
                            false,
                        ),
                        Err(error) => (format!("注册失败：{error:#}"), true),
                    });
                }
                return;
            }
            if self.browsers.len() > 1 {
                let old = self.profile.clone();
                let current = self
                    .browsers
                    .keys()
                    .position(|id| Some(id) == self.profile.as_ref());
                egui::ComboBox::from_label("扫描范围")
                    .selected_text(current.map_or_else(
                        || "所有已连接的 Edge 配置".into(),
                        |index| format!("Edge 配置 {}", index + 1),
                    ))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.profile, None, "所有已连接的 Edge 配置");
                        for (index, profile) in self.browsers.keys().enumerate() {
                            ui.selectable_value(
                                &mut self.profile,
                                Some(profile.clone()),
                                format!("Edge 配置 {}", index + 1),
                            );
                        }
                    });
                if old != self.profile {
                    self.previews.clear();
                }
            }
            ui.horizontal(|ui| {
                if ui.button("重新扫描").clicked() {
                    self.start(Job::Scan(browsers.to_vec()), "正在扫描 Edge…");
                }
                if ui.button("导出本地快照").clicked() {
                    let result: Result<Vec<_>> = browsers
                        .iter()
                        .map(|item| recovery::export(&item.snapshot))
                        .collect();
                    self.status = Some(match result {
                        Ok(paths) => (
                            format!("快照已保存到 {}", paths[0].parent().unwrap().display()),
                            false,
                        ),
                        Err(error) => (format!("导出失败：{error:#}"), true),
                    });
                }
            });
            egui::CollapsingHeader::new("查看扫描到的标签页").show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("inventory")
                    .max_height(190.0)
                    .show(ui, |ui| {
                        for browser in browsers {
                            let urls = url_counts(&browser.snapshot);
                            for tab in &browser.snapshot.tabs {
                                let prefix = format!("窗口 {} · ", tab.window_id);
                                let display = if tab.title.is_empty() {
                                    "未命名标签页"
                                } else {
                                    &tab.title
                                };
                                let mut text = match &tab.protection {
                                    Some(reason) => format!("{prefix}{display}  ·  {reason}"),
                                    None => format!("{prefix}{display}"),
                                };
                                if urls.get(tab.url.as_str()).is_some_and(|&count| count > 1) {
                                    text.push_str("  ·  重复网址");
                                }
                                ui.add(egui::Label::new(text).truncate())
                                    .on_hover_text(&tab.url);
                            }
                        }
                    });
            });
        });
        let rect = stage.content_ui.min_rect().expand(16.0);
        stage
            .content_ui
            .painter()
            .set(backdrop, theme::atmosphere(rect));
        stage.end(ui);
    }

    fn options(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.label(RichText::new("02  /  整理偏好").size(12.0).color(SUBTLE));
            ui.label(RichText::new("本地分类 · 目标窗口数量").size(17.0));
            ui.horizontal(|ui| {
                let width = ((ui.available_width() - 5.0 * 10.0) / 6.0).min(64.0);
                for number in 1..=6 {
                    if theme::choice(
                        ui,
                        &number.to_string(),
                        self.windows == number,
                        egui::vec2(width, 40.0),
                    )
                    .clicked()
                    {
                        self.windows = number;
                        self.previews.clear();
                    }
                }
            });
            ui.label(RichText::new("分类精细度").size(17.0));
            ui.horizontal_wrapped(|ui| {
                for (value, label) in Detail::OPTIONS {
                    if theme::choice(ui, label, self.detail == value, egui::vec2(0.0, 38.0))
                        .clicked()
                    {
                        self.detail = value;
                        self.previews.clear();
                    }
                }
            });
            ui.label(
                RichText::new("已有手动组保留。受保护的标签页和同组成员留在原处。").color(SUBTLE),
            );
        });
    }

    fn preview(&self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.label(RichText::new("03  /  分类预览").size(12.0).color(SUBTLE));
            ui.label(RichText::new("工作场景，一目了然。").size(22.0));
            if self.previews.is_empty() {
                ui.label(
                    RichText::new("生成预览后，这里会显示窗口的工作场景和各组用途。").color(SUBTLE),
                );
                return;
            }
            for (profile_index, item) in self.previews.iter().enumerate() {
                let urls = url_counts(&item.snapshot);
                if self.previews.len() > 1 {
                    ui.strong(format!("Edge 配置 {}", profile_index + 1));
                }
                ui.label(format!(
                    "{} 个目标窗口   ·   {} 页留在原处   ·   {} 个重复网址",
                    item.plan.windows.len(),
                    item.plan.protected_count,
                    item.plan.duplicate_count
                ));
                if item.ai.is_none() && item.plan.windows.len() < self.windows {
                    ui.label(RichText::new("可用主题较少，预览保留完整的主题组。").color(SUBTLE));
                }
                if let Some(summary) = item.ai {
                    let groups: usize = item
                        .plan
                        .windows
                        .iter()
                        .map(|window| window.groups.len())
                        .sum();
                    ui.label(
                        RichText::new(format!(
                            "AI 预览 · 共 {groups} 组 · 将重新分类 {} 个已选原组",
                            item.plan.split_group_ids.len()
                        ))
                        .color(SUBTLE),
                    );
                    if summary.uncertain_tabs > 0 {
                        ui.label(format!(
                            "{} 页用途线索不足，已放入「无法确定」。",
                            summary.uncertain_tabs
                        ));
                    }
                    if summary.unassigned_tabs > 0 {
                        ui.label(format!(
                            "{} 页没有唯一的 AI 归类，已放入「待确认 · AI 未归类」。",
                            summary.unassigned_tabs
                        ));
                    }
                    if summary.unassigned_groups > 0 {
                        ui.label(format!(
                            "{} 个原组没有唯一的 AI 归属，已整组保留并安排窗口。",
                            summary.unassigned_groups
                        ));
                    }
                }
                let by_id = item
                    .snapshot
                    .tabs
                    .iter()
                    .map(|tab| (tab.id, tab))
                    .collect::<BTreeMap<_, _>>();
                for (index, target) in item.plan.windows.iter().enumerate() {
                    egui::Frame::new()
                        .fill(theme::STONE)
                        .corner_radius(16)
                        .inner_margin(14)
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.label(
                                RichText::new(format!(
                                    "窗口 {} · {}（{} 组）",
                                    index + 1,
                                    target.title,
                                    target.groups.len()
                                ))
                                .color(ACCENT)
                                .strong(),
                            );
                            for (group_index, group) in target.groups.iter().enumerate() {
                                let label = if group.title.is_empty() {
                                    "未命名手动组"
                                } else {
                                    &group.title
                                };
                                let kind = if group.existing_group_id.is_some() {
                                    " · 保留原组"
                                } else {
                                    ""
                                };
                                egui::CollapsingHeader::new(format!(
                                    "{label} · {} 页{kind}",
                                    group.tab_ids.len()
                                ))
                                .id_salt((profile_index, index, group_index))
                                .show(ui, |ui| {
                                    for id in &group.tab_ids {
                                        let tab = by_id[id];
                                        let title = if tab.title.is_empty() {
                                            &tab.url
                                        } else {
                                            &tab.title
                                        };
                                        let suffix = if urls
                                            .get(tab.url.as_str())
                                            .is_some_and(|&count| count > 1)
                                        {
                                            "  ·  重复网址"
                                        } else {
                                            ""
                                        };
                                        ui.add(
                                            egui::Label::new(format!("{title}{suffix}")).truncate(),
                                        )
                                        .on_hover_text(&tab.url);
                                    }
                                });
                            }
                        });
                }
            }
        });
    }

    fn show_ai_dialog(&mut self, context: &egui::Context) {
        let Some(dialog) = self.ai_dialog.as_mut() else {
            return;
        };
        let mut start = None;
        let mut cancel = false;
        let modal = egui::Modal::new(egui::Id::new("ai-analysis")).frame(theme::modal_frame()).show(context, |ui| {
            let width = (context.content_rect().width() - 100.0).clamp(270.0, 480.0);
            ui.set_width(width);
            let height = (context.content_rect().height() - 140.0).clamp(260.0, 690.0);
            ui.set_min_height(height);
            egui::ScrollArea::vertical().id_salt("ai-dialog-content").max_height(height).min_scrolled_height(height).auto_shrink([false, false]).show(ui, |ui| {
            ui.label(RichText::new("AI WORKSPACE").size(12.0).color(SUBTLE));
            ui.heading("给页面找到合适的位置");
            ui.label("AI 会优先为页面匹配主题，并推荐窗口数和标签组数。");
            ui.horizontal(|ui| {
                if ui.button("DeepSeek").clicked() {
                    self.ai_endpoint = ai::DEEPSEEK_ENDPOINT.into();
                    self.ai_model = ai::DEEPSEEK_MODEL.into();
                    self.ai_key.clear();
                }
                if ui.button("本机模型").clicked() {
                    self.ai_endpoint = ai::LOCAL_ENDPOINT.into();
                    self.ai_model.clear();
                    self.ai_key.clear();
                }
            });
            ui.label("AI 服务地址");
            if ui.add(egui::TextEdit::singleline(&mut self.ai_endpoint).desired_width(f32::INFINITY)).changed() {
                self.ai_key.clear();
            }
            ui.label("模型名称");
            ui.add(egui::TextEdit::singleline(&mut self.ai_model).desired_width(f32::INFINITY));
            ui.label("API 密钥 · 本机接口可留空");
            ui.add(egui::TextEdit::singleline(&mut self.ai_key).password(true).desired_width(f32::INFINITY));
            ui.separator();
            let snapshot = &dialog.browser.snapshot;
            let eligible = if dialog.browser.group_split { ai::splittable_group_ids(snapshot) } else { BTreeSet::new() };
            let all_selected = !eligible.is_empty() && eligible.is_subset(&dialog.split);
            ui.horizontal_wrapped(|ui| {
                ui.strong("勾选要拆开重新分析的原组");
                let caption = if all_selected { "取消全选" } else { "全选" };
                if ui.add_enabled(!eligible.is_empty(), egui::Button::new(caption)).on_hover_text("仅勾选可以拆分的原组").clicked() {
                    if all_selected { dialog.split.clear(); } else { dialog.split.clone_from(&eligible); }
                }
                ui.small(format!("已选 {} 组", dialog.split.len()));
            });
            ui.label("未勾选的原组按整体安排窗口。含受保护标签的组留在原处。");
            if !dialog.browser.group_split {
                ui.label(RichText::new("请更新并重新加载包内 Edge 扩展，再选择要拆分的原组。").color(SUBTLE));
            }
            egui::ScrollArea::vertical().id_salt("ai-groups").max_height(130.0).show(ui, |ui| {
                if snapshot.groups.is_empty() { ui.label("没有已有分组，AI 将分析可移动的未分组标签页。"); }
                for group in &snapshot.groups {
                    let count = snapshot.tabs.iter().filter(|tab| tab.group_id == group.id).count();
                    let name = if group.title.is_empty() { "未命名组" } else { &group.title };
                    let mut selected = dialog.split.contains(&group.id);
                    if ui.add_enabled(eligible.contains(&group.id), egui::Checkbox::new(&mut selected, format!("{name}（{count} 页）"))).changed() {
                        if selected { dialog.split.insert(group.id); } else { dialog.split.remove(&group.id); }
                    }
                }
            });
            egui::CollapsingHeader::new("分析会发送哪些信息？").show(ui, |ui| {
                ui.label(RichText::new("将把可移动页面的标题、用途提示和临时编号发送给所填接口。未拆分原组只附组名和最多 4 个标题样本。标题可能含私人信息，远程服务可能计费。分析完成后先显示预览。").color(SUBTLE));
            });
            ui.label(RichText::new("确认后，页面标题会发送给所填 AI 服务，远程分析可能计费。").size(12.0).color(SUBTLE));
            if let Some(error) = &dialog.error { ui.label(RichText::new(error).color(Color32::from_rgb(150, 47, 31))); }
            ui.horizontal(|ui| {
                if ui.add(theme::primary("确认发送并分析")).clicked() {
                    match ai::Settings::new(&self.ai_endpoint, &self.ai_model, &self.ai_key) {
                        Ok(settings) => start = Some(Job::AiPreview(dialog.browser.clone(), settings, dialog.split.clone())),
                        Err(error) => dialog.error = Some(error.to_string()),
                    }
                }
                cancel = ui.button("取消").clicked();
            });
            });
        });
        if start.is_some() || cancel || modal.should_close() {
            self.ai_dialog = None;
        }
        if let Some(job) = start {
            self.start(job, "AI 正在分析标题与用途，浏览器布局尚未改动…");
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        if self.busy
            && !self.ai_running
            && context.input(|input| input.viewport().close_requested())
        {
            context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        context.request_repaint_after(Duration::from_millis(if self.busy { 70 } else { 350 }));
        let browsers = self.selected();
        egui::TopBottomPanel::bottom("actions")
            .frame(egui::Frame::new().fill(theme::CANVAS).inner_margin(20))
            .show(context, |ui| {
                ui.separator();
                if let Some((status, error)) = &self.status {
                    ui.label(RichText::new(status).color(if *error {
                        Color32::from_rgb(150, 47, 31)
                    } else {
                        SUBTLE
                    }));
                }
                ui.add_enabled_ui(!self.busy, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        let has_plan = self
                            .previews
                            .iter()
                            .any(|item| !item.plan.windows.is_empty());
                        let caption = if has_plan {
                            "确定整理"
                        } else {
                            "生成预览"
                        };
                        let button = theme::primary(caption);
                        if ui.add_enabled(!browsers.is_empty(), button).clicked() {
                            if has_plan {
                                self.start(
                                    Job::Execute(self.previews.clone()),
                                    "正在保存快照并整理 Edge…",
                                );
                            } else {
                                self.start(
                                    Job::Preview(browsers.clone(), self.windows, self.detail),
                                    "正在本地分类…",
                                );
                            }
                        }
                        let ai_button = ui
                            .add_enabled(
                                browsers.len() == 1,
                                egui::Button::new("AI 分析")
                                    .fill(theme::LILAC)
                                    .stroke(egui::Stroke::NONE)
                                    .corner_radius(16)
                                    .min_size(egui::vec2(102.0, 44.0)),
                            )
                            .on_disabled_hover_text(
                                "AI 每次分析一个 Edge 配置。请先选择一个配置。",
                            );
                        if ai_button.clicked() {
                            self.ai_dialog = Some(AiDialog {
                                browser: browsers[0].clone(),
                                split: BTreeSet::new(),
                                error: None,
                            });
                        }
                        let has_recovery = browsers
                            .iter()
                            .any(|item| recovery::available(&item.snapshot.profile_id));
                        if ui
                            .add_enabled(has_recovery, egui::Button::new("恢复布局"))
                            .clicked()
                        {
                            self.restore_confirm = true;
                        }
                        if self.busy {
                            ui.spinner();
                        }
                    });
                });
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::CANVAS).inner_margin(24))
            .show(context, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.intro(ui);
                        ui.add_space(5.0);
                        ui.add_enabled_ui(!self.busy, |ui| {
                            self.connection(ui, &browsers);
                            self.options(ui);
                        });
                        self.preview(ui);
                    });
            });
        if self.restore_confirm {
            let mut confirmed = false;
            let mut cancel = false;
            let modal = egui::Modal::new(egui::Id::new("confirm-restore")).frame(theme::modal_frame()).show(context, |ui| {
                ui.set_max_width(360.0);
                ui.heading("恢复最近一次整理前的布局");
                ui.label("会优先移动仍打开的原标签页，必要时重新打开原网址。网页滚动位置、未提交表单和页面临时状态可能无法恢复。整理后新开的页面会保留。");
                ui.horizontal(|ui| {
                    confirmed = ui.button("恢复布局").clicked();
                    cancel = ui.button("取消").clicked();
                });
            });
            if confirmed || cancel || modal.should_close() {
                self.restore_confirm = false;
            }
            if confirmed {
                self.start(Job::Restore(browsers), "正在恢复布局…");
            }
        }
        self.show_ai_dialog(context);
    }
}

fn url_counts(snapshot: &Snapshot) -> BTreeMap<&str, usize> {
    let mut counts = BTreeMap::new();
    for tab in snapshot.tabs.iter().filter(|tab| !tab.url.is_empty()) {
        *counts.entry(tab.url.as_str()).or_default() += 1;
    }
    counts
}

fn scan(mut browser: Browser) -> Result<Browser> {
    browser.snapshot = serde_json::from_value(browser.request("scan", Value::Null)?)?;
    Ok(browser)
}

fn perform(job: Job, model: &mut Option<semantic::Model>) -> Result<Output> {
    match job {
        Job::Scan(browsers) => {
            let updates = browsers.into_iter().map(scan).collect::<Result<Vec<_>>>()?;
            Ok(Output::Browsers(updates, "扫描已更新。".into()))
        }
        Job::Preview(browsers, windows, detail) => {
            if model.is_none() {
                *model = Some(semantic::Model::load(&crate::model_directory()?)?);
            }
            let model = model.as_mut().context("无法加载分类器")?;
            let mut previews = Vec::new();
            for browser in browsers.into_iter().map(scan) {
                let browser = browser?;
                let snapshot = browser.snapshot.clone();
                if snapshot.tabs.len() > 1500 {
                    bail!("当前版本每个 Edge 配置最多预览 1500 个标签页。");
                }
                let plan = planner::preview(&snapshot, model, windows, detail)?;
                previews.push(Preview {
                    browser,
                    snapshot,
                    plan,
                    ai: None,
                });
            }
            Ok(Output::Preview(previews))
        }
        Job::AiPreview(browser, settings, split) => {
            if !split.is_empty() && !browser.group_split {
                bail!("请更新 Edge 连接扩展后再拆分原组。");
            }
            let live = scan(browser.clone())?;
            if live.snapshot.revision != browser.snapshot.revision {
                bail!("选择后 Edge 布局已变化。请重新扫描并选择原组。");
            }
            let snapshot = live.snapshot.clone();
            let (plan, summary) = ai::preview(&snapshot, &settings, &split, model)?;
            Ok(Output::Preview(vec![Preview {
                browser: live,
                snapshot,
                plan,
                ai: Some(summary),
            }]))
        }
        Job::Execute(previews) => {
            let actionable = previews
                .into_iter()
                .filter(|item| !item.plan.windows.is_empty())
                .collect::<Vec<_>>();
            for item in &actionable {
                let live = scan(item.browser.clone())?;
                if item.snapshot.revision != live.snapshot.revision {
                    bail!("预览后 Edge 布局已变化。请重新生成预览。");
                }
            }
            let mut updates = Vec::new();
            let mut moved = 0;
            for item in actionable {
                recovery::save(&item.snapshot).context("快照保存失败，已停止整理")?;
                let response = item
                    .browser
                    .request("execute", serde_json::to_value(&item.plan)?)
                    .context("整理已停止。可使用「恢复布局」查看最近一次整理前的布局")?;
                moved += response["moved"].as_u64().unwrap_or(0);
                let mut browser = item.browser;
                browser.snapshot = serde_json::from_value(response["snapshot"].clone())?;
                updates.push(browser);
            }
            Ok(Output::Browsers(
                updates,
                format!("已整理 {moved} 个标签页。可恢复最近一次整理前的布局。"),
            ))
        }
        Job::Restore(browsers) => {
            let mut updates = Vec::new();
            let (mut restored, mut reopened, mut skipped) = (0, 0, 0);
            for mut browser in browsers
                .into_iter()
                .filter(|item| recovery::available(&item.snapshot.profile_id))
            {
                let saved = recovery::load(&browser.snapshot.profile_id)?;
                let response = browser.request("restore", serde_json::to_value(saved)?)?;
                restored += response["restored"].as_u64().unwrap_or(0);
                reopened += response["reopened"].as_u64().unwrap_or(0);
                let skipped_in_profile = response["skipped"]
                    .as_u64()
                    .context("恢复结果缺少跳过数量")?;
                skipped += skipped_in_profile;
                browser.snapshot = serde_json::from_value(response["snapshot"].clone())?;
                if skipped_in_profile == 0 {
                    recovery::remove(&browser.snapshot.profile_id)?;
                }
                updates.push(browser);
            }
            Ok(Output::Browsers(updates, format!("已恢复 {restored} 页，其中 {reopened} 页重新打开，{skipped} 页因保护或限制跳过。")))
        }
    }
}

#[cfg(test)]
mod edge_test {
    use super::*;

    #[test]
    #[ignore = "launched by scripts/test-edge.mjs with an isolated Edge profile"]
    fn isolated_edge_round_trip() -> Result<()> {
        let directory = recovery::data_directory()?;
        let url = std::env::var("TABSORT_TEST_URL")?;
        let models = std::env::var("TABSORT_TEST_MODELS")?;
        let _isolated_pipe = std::env::var("TABSORT_TEST_PIPE")?;
        let _isolated_data = std::env::var("TABSORT_TEST_DATA")?;
        let events = bridge::listen();
        let started = std::time::Instant::now();
        let browser = loop {
            match events.recv_timeout(Duration::from_secs(60))? {
                bridge::Event::Connected(browser) => break browser,
                bridge::Event::ListenerError(message) => bail!(message),
                bridge::Event::Disconnected(_) => (),
            }
        };
        let scan_ms = started.elapsed().as_millis();
        anyhow::ensure!(
            browser.snapshot.tabs.len() == 12
                && browser
                    .snapshot
                    .tabs
                    .iter()
                    .all(|tab| tab.url.starts_with(&url)),
            "未找到 12 个隔离测试页"
        );
        let mut model = Some(semantic::Model::load(std::path::Path::new(&models))?);
        let Output::Preview(items) = perform(
            Job::Preview(vec![browser.clone()], 2, Detail::Standard),
            &mut model,
        )?
        else {
            bail!("预览没有结果");
        };
        let before = items[0].snapshot.clone();
        let mut stale = items[0].plan.clone();
        stale.revision.clear();
        let rejected = browser
            .request("execute", serde_json::to_value(stale)?)
            .unwrap_err();
        anyhow::ensure!(rejected.to_string().contains("重新扫描"));
        anyhow::ensure!(scan(browser.clone())?.snapshot.revision == before.revision);
        let Output::Browsers(after, _) = perform(Job::Execute(items), &mut model)? else {
            bail!("整理没有结果");
        };
        anyhow::ensure!(recovery::available(&before.profile_id));
        let after = &after[0].snapshot;
        let original_group = before.groups.first().context("缺少手动测试组")?;
        anyhow::ensure!(after
            .groups
            .iter()
            .any(|group| group.title == original_group.title
                && group.color == original_group.color
                && group.collapsed == original_group.collapsed));
        let pinned = before
            .tabs
            .iter()
            .find(|tab| tab.pinned)
            .context("缺少固定测试页")?;
        anyhow::ensure!(after
            .tabs
            .iter()
            .any(|tab| tab.id == pinned.id && tab.window_id == pinned.window_id && tab.pinned));
        let Output::Browsers(restored, message) =
            perform(Job::Restore(vec![browser.clone()]), &mut model)?
        else {
            bail!("恢复没有结果");
        };
        let restored = &restored[0].snapshot;
        let layout = |snapshot: &Snapshot| {
            let mut windows = snapshot
                .windows
                .iter()
                .map(|window| {
                    snapshot
                        .tabs
                        .iter()
                        .filter(|tab| tab.window_id == window.id)
                        .map(|tab| {
                            (
                                tab.id,
                                tab.url.clone(),
                                tab.pinned,
                                snapshot
                                    .groups
                                    .iter()
                                    .find(|group| group.id == tab.group_id)
                                    .map(|group| {
                                        (group.title.clone(), group.color.clone(), group.collapsed)
                                    }),
                            )
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            windows.sort();
            windows
        };
        anyhow::ensure!(
            layout(restored) == layout(&before),
            "恢复后布局不同：\n之前 {:?}\n之后 {:?}",
            layout(&before),
            layout(restored)
        );
        anyhow::ensure!(!recovery::available(&before.profile_id));
        let browser = scan(browser)?;
        let ai_before = browser.snapshot.clone();
        let selected = ai_before
            .groups
            .iter()
            .find(|group| group.title == "手动研究组")
            .context("缺少待拆分测试组")?;
        let retained = ai_before
            .groups
            .iter()
            .find(|group| group.title == "保留的开发组")
            .context("缺少保留测试组")?;
        let settings = ai::Settings::new(
            &format!("{url}v1/chat/completions"),
            "fixture-model",
            "fixture-key",
        )?;
        let Output::Preview(ai_previews) = perform(
            Job::AiPreview(browser.clone(), settings, BTreeSet::from([selected.id])),
            &mut model,
        )?
        else {
            bail!("AI 预览没有结果");
        };
        let plan = &ai_previews[0].plan;
        let summary = ai_previews[0].ai.context("缺少 AI 覆盖统计")?;
        anyhow::ensure!(
            plan.windows.len() == 3
                && summary.unassigned_tabs == 2
                && summary.unassigned_groups == 1
                && summary.uncertain_tabs == 1
        );
        let uncertain = plan
            .windows
            .iter()
            .flat_map(|window| &window.groups)
            .find(|group| group.title == "无法确定")
            .context("缺少无法确定组")?;
        let uncertain_ids: BTreeSet<_> = ai_before
            .tabs
            .iter()
            .filter(|tab| tab.title == "internship")
            .map(|tab| tab.id)
            .collect();
        anyhow::ensure!(
            uncertain.tab_ids.iter().copied().collect::<BTreeSet<_>>() == uncertain_ids
        );
        let dashboard = ai_before
            .tabs
            .iter()
            .find(|tab| tab.title == "dashboard")
            .context("缺少通用标题测试页")?;
        anyhow::ensure!(plan
            .windows
            .iter()
            .flat_map(|window| &window.groups)
            .any(|group| group.title == "辅助工作" && group.tab_ids.contains(&dashboard.id)));
        let assigned: Vec<_> = plan
            .windows
            .iter()
            .flat_map(|window| &window.groups)
            .flat_map(|group| group.tab_ids.iter().copied())
            .collect();
        let expected: BTreeSet<_> = ai_before
            .tabs
            .iter()
            .filter(|tab| tab.protection.is_none())
            .map(|tab| tab.id)
            .collect();
        anyhow::ensure!(
            assigned.len() == expected.len()
                && assigned.into_iter().collect::<BTreeSet<_>>() == expected
        );
        anyhow::ensure!(plan.split_group_ids == [selected.id]);
        anyhow::ensure!(plan
            .windows
            .iter()
            .flat_map(|window| &window.groups)
            .any(|group| group.existing_group_id == Some(retained.id)));
        anyhow::ensure!(scan(browser.clone())?.snapshot.revision == ai_before.revision);
        let mut unauthorized = plan.clone();
        unauthorized.split_group_ids.clear();
        let rejected = browser
            .request("execute", serde_json::to_value(unauthorized)?)
            .unwrap_err();
        anyhow::ensure!(rejected.to_string().contains("未经授权"));
        anyhow::ensure!(scan(browser.clone())?.snapshot.revision == ai_before.revision);
        let Output::Browsers(ai_after, _) = perform(Job::Execute(ai_previews), &mut model)? else {
            bail!("AI 整理没有结果");
        };
        let ai_after = &ai_after[0].snapshot;
        anyhow::ensure!(ai_after.tabs.len() == ai_before.tabs.len());
        anyhow::ensure!(!ai_after
            .groups
            .iter()
            .any(|group| group.title == selected.title));
        anyhow::ensure!(ai_after
            .groups
            .iter()
            .any(|group| group.title == retained.title && group.color == retained.color));
        let Output::Browsers(ai_restored, _) = perform(Job::Restore(vec![browser]), &mut model)?
        else {
            bail!("AI 整理后的恢复没有结果");
        };
        anyhow::ensure!(
            layout(&ai_restored[0].snapshot) == layout(&ai_before),
            "拆组后恢复未保留原布局：\n之前 {:?}\n之后 {:?}",
            layout(&ai_before),
            layout(&ai_restored[0].snapshot)
        );
        std::fs::write(
            directory.join("edge-result.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "tabs": before.tabs.len(), "scanMsIncludingConnectWait": scan_ms, "stalePlanRejected": true, "pinnedRetained": true,
                "manualGroupRetained": true, "originalTabIdsAndLayoutRestored": true, "result": message,
                "mockAiOptInSplitAndRestore": true, "unauthorizedGroupSplitRejected": true, "uncertainTabsGroupedSeparately": true, "genericTitleCanBeClassified": true,
            }))?,
        )?;
        Ok(())
    }
}
