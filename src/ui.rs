mod history;
mod journal_dates;
mod management;
use crate::glass;
use crate::{
    db::{self, Host},
    ssh,
    worker::{self, Action, Worker},
};
use anyhow::Result;
use egui::{self, Color32 as C, Frame, RichText as T, Ui};
use rusqlite::{Connection, params};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::mpsc,
    thread,
    time::Duration,
};

const BG: C = C::from_rgb(28, 30, 32);
const SIDE: C = C::from_rgb(34, 36, 38);
const CARD: C = C::from_rgb(38, 41, 43);
const LINE: C = C::from_rgb(69, 74, 78);
const MUTED: C = C::from_rgb(178, 185, 190);
const MINT: C = glass::ACCENT;
fn card() -> Frame {
    Frame::new()
        .fill(CARD)
        .stroke((1., LINE))
        .corner_radius(11)
        .inner_margin(18)
}
fn title(ui: &mut Ui, s: &str) {
    ui.label(T::new(s).size(26.).strong());
}
fn hint(ui: &mut Ui, s: &str) {
    ui.label(T::new(s).color(MUTED));
}
fn main_button(ui: &mut Ui, s: &str) -> bool {
    ui.add(
        egui::Button::new(T::new(s).color(BG).strong())
            .fill(MINT)
            .corner_radius(8.)
            .min_size(egui::vec2(140., 34.)),
    )
    .clicked()
}
fn field(ui: &mut Ui, value: &mut String, placeholder: &str, secret: bool) -> egui::Response {
    ui.add(
        egui::TextEdit::singleline(value)
            .password(secret)
            .hint_text(T::new(placeholder).color(C::from_rgba_unmultiplied(231, 237, 240, 128)))
            .margin(egui::Margin::symmetric(10, 7))
            .desired_width(f32::INFINITY),
    )
}
fn is_online(status: &str) -> bool {
    status.starts_with("На связи") || status.starts_with("Обновлено")
}
fn order_hosts(db: &Connection, hosts: &mut [Host]) {
    let raw: rusqlite::Result<String> = db.query_row(
        "SELECT value FROM panel_settings WHERE key='server_order'",
        [],
        |r| r.get(0),
    );
    if let Ok(order) = raw.and_then(|value| {
        serde_json::from_str::<Vec<String>>(&value)
            .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))
    }) {
        let positions: HashMap<_, _> = order
            .iter()
            .enumerate()
            .map(|(i, name)| (name, i))
            .collect();
        hosts.sort_by_key(|host| positions.get(&host.name).copied().unwrap_or(usize::MAX));
    }
}
fn data_size(bytes: f64) -> String {
    for (scale, unit) in [(1073741824., "ГиБ"), (1048576., "МиБ"), (1024., "КиБ")] {
        if bytes >= scale {
            return format!("{:.1} {unit}", bytes / scale);
        }
    }
    format!("{bytes:.0} Б")
}
fn meter(ui: &mut Ui, label: &str, fraction: Option<f64>, detail: String) {
    ui.vertical(|ui| {
        let value = fraction.map_or("—".into(), |value| format!("{:.0}%", value * 100.));
        ui.label(T::new(format!("{label}  {value}  {detail}")).size(13.));
        ui.add(
            egui::ProgressBar::new(fraction.unwrap_or_default() as f32)
                .desired_width(135.)
                .desired_height(4.)
                .fill(glass::CYAN),
        );
    });
}
fn when(t: i64) -> String {
    chrono::DateTime::from_timestamp(t, 0)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%d.%m  %H:%M:%S")
                .to_string()
        })
        .unwrap_or_default()
}
#[derive(PartialEq, Clone, Copy, Hash)]
enum Page {
    Overview,
    Servers,
    Journal,
    Analytics,
}
pub struct App {
    updater: crate::update::Updater,
    _profile_lock: std::fs::File,
    logo: egui::TextureHandle,
    db: Connection,
    path: PathBuf,
    hosts: Vec<Host>,
    workers: HashMap<String, Worker>,
    status: HashMap<String, String>,
    update_clock: HashMap<String, (std::time::Instant, std::time::Instant)>,
    page: Page,
    pair_open: bool,
    name: String,
    code: String,
    pair_result: Option<mpsc::Receiver<Result<String>>>,
    error: String,
    selected: HashSet<String>,
    remove: Vec<String>,
    detail: Option<String>,
    day: bool,
    packet_hours: Option<i64>,
    dragging: Option<String>,
    filter: String,
    scope: String,
    attack_filter: String,
    offset: i64,
    journal_dates: journal_dates::DateFilter,
    management: management::Management,
    edit: Option<(String, String, String, u16)>,
    saving_profile: bool,
}
impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Result<Self> {
        if let Some(font) = std::env::var_os("WINDIR")
            .and_then(|d| std::fs::read(PathBuf::from(d).join("Fonts/segoeui.ttf")).ok())
        {
            let mut fonts = egui::FontDefinitions::default();
            fonts
                .font_data
                .insert("Segoe UI".into(), egui::FontData::from_owned(font).into());
            fonts
                .families
                .get_mut(&egui::FontFamily::Proportional)
                .unwrap()
                .insert(0, "Segoe UI".into());
            cc.egui_ctx.set_fonts(fonts);
        }
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = BG;
        style.visuals.window_fill = SIDE;
        style.visuals.override_text_color = Some(C::from_rgb(231, 237, 240));
        style.visuals.widgets.inactive.bg_fill = CARD;
        style.visuals.selection.bg_fill = C::from_rgb(63, 84, 96);
        style.visuals.widgets.hovered.bg_fill = C::from_rgb(58, 64, 68);
        style.visuals.widgets.hovered.weak_bg_fill = C::from_rgb(58, 64, 68);
        style.visuals.widgets.active.bg_fill = C::from_rgb(69, 78, 83);
        style.visuals.extreme_bg_color = C::from_rgb(24, 26, 28);
        style.visuals.window_corner_radius = 8.into();
        style.visuals.window_stroke = egui::Stroke::new(1_f32, LINE);
        style.visuals.widgets.inactive.weak_bg_fill = C::from_rgb(49, 53, 56);
        style.visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1_f32, LINE);
        style.visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1_f32, MUTED);
        style.visuals.selection.stroke = egui::Stroke::new(1.5_f32, glass::ACCENT);
        for widget in [
            &mut style.visuals.widgets.noninteractive,
            &mut style.visuals.widgets.inactive,
            &mut style.visuals.widgets.hovered,
            &mut style.visuals.widgets.active,
            &mut style.visuals.widgets.open,
        ] {
            widget.corner_radius = 9.into();
        }
        style.spacing.button_padding = egui::vec2(12., 7.);
        style.spacing.interact_size.y = 32.;
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(15.));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(14.5));
        style.spacing.item_spacing = egui::vec2(10., 9.);
        cc.egui_ctx.set_style(style);
        let path = db::path()?;
        let profile_lock = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(path.with_extension("lock"))?;
        profile_lock
            .try_lock()
            .map_err(|_| anyhow::anyhow!("Этот профиль уже открыт в другом окне"))?;
        let icon = eframe::icon_data::from_png_bytes(include_bytes!("rkn-mark.png"))?;
        let logo = cc.egui_ctx.load_texture(
            "rkn-mark",
            egui::ColorImage::from_rgba_unmultiplied(
                [icon.width as usize, icon.height as usize],
                &icon.rgba,
            ),
            Default::default(),
        );
        let db = db::open(&path)?;
        let mut hosts = db::hosts(&db)?;
        order_hosts(&db, &mut hosts);
        let mut app = Self {
            updater: crate::update::Updater::start(),
            _profile_lock: profile_lock,
            logo,
            db,
            path,
            hosts,
            workers: HashMap::new(),
            status: HashMap::new(),
            update_clock: HashMap::new(),
            page: Page::Overview,
            pair_open: false,
            name: String::new(),
            code: String::new(),
            pair_result: None,
            error: String::new(),
            selected: HashSet::new(),
            remove: Vec::new(),
            detail: None,
            day: false,
            packet_hours: None,
            dragging: None,
            filter: String::new(),
            scope: String::new(),
            attack_filter: String::new(),
            offset: 0,
            journal_dates: Default::default(),
            management: Default::default(),
            edit: None,
            saving_profile: false,
        };
        for h in app.hosts.clone() {
            if h.active {
                app.start(&h)
            }
        }
        Ok(app)
    }
    fn start(&mut self, h: &Host) {
        if !self.workers.contains_key(&h.name) {
            self.workers
                .insert(h.name.clone(), worker::launch(self.path.clone(), h.clone()));
            self.status.insert(h.name.clone(), "Подключение…".into());
        }
    }
    fn dispatch(&mut self) {
        self.workers.retain(|name, w| {
            while let Ok((rules, result)) = w.catalogs.try_recv() {
                let cache = if rules {
                    &mut self.management.rules
                } else {
                    &mut self.management.catalogs
                };
                cache.insert(name.clone(), Some(result));
            }
            while let Ok(result) = w.results.try_recv() {
                self.management.finish(name, result);
            }
            loop {
                match w.status.try_recv() {
                    Ok(s) => {
                        if s.starts_with("Обновление · ") {
                            let now = std::time::Instant::now();
                            let clock = self.update_clock.entry(name.clone()).or_insert((now, now));
                            if self.status.get(name) != Some(&s) {
                                clock.0 = now;
                            }
                            clock.1 = now;
                        } else {
                            self.update_clock.remove(name);
                        }
                        self.status.insert(name.clone(), s);
                    }
                    Err(mpsc::TryRecvError::Empty) => return true,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        while let Ok(result) = w.results.try_recv() {
                            self.management.finish(name, result);
                        }
                        self.management.disconnected(name);
                        if self.status.get(name).is_some_and(|s| s == "Остановка…") {
                            self.status.remove(name);
                        }
                        return false;
                    }
                }
            }
        });
        if let Some(result) = self.pair_result.as_ref().and_then(|r| r.try_recv().ok()) {
            self.pair_result = None;
            match db::hosts(&self.db) {
                Ok(mut hosts) => {
                    order_hosts(&self.db, &mut hosts);
                    self.hosts = hosts;
                }
                Err(e) => self.error = e.to_string(),
            }
            match result {
                Ok(n) => {
                    self.pair_open = false;
                    self.code.clear();
                    self.error.clear();
                    self.page = Page::Servers;
                    if let Some(h) = self.hosts.iter().find(|h| h.name == n).cloned() {
                        self.start(&h)
                    }
                }
                Err(e) => self.error = format!("{e:#}"),
            }
        }
    }
    fn open_profile(&mut self, host: &Host) {
        self.error.clear();
        self.edit = Some((
            host.name.clone(),
            host.name.clone(),
            host.addr.clone(),
            host.port,
        ));
        self.remove.clear();
    }

    fn finish_profile_edit(&mut self, ctx: &egui::Context) {
        if !self.saving_profile {
            return;
        }
        let Some((original, name, address, port)) = self.edit.as_ref() else {
            self.saving_profile = false;
            return;
        };
        if self.workers.contains_key(original) {
            return;
        }
        let order = self
            .hosts
            .iter()
            .map(|h| {
                if h.name == *original {
                    name.as_str()
                } else {
                    h.name.as_str()
                }
            })
            .collect::<Vec<_>>();
        let result = db::update_profile(&mut self.db, original, name, address, *port, &order);
        self.saving_profile = false;
        match result {
            Ok(active) => {
                let (original, name, address, port) = self.edit.take().unwrap();
                let Some(host) = self.hosts.iter_mut().find(|h| h.name == original) else {
                    return;
                };
                host.name = name.clone();
                host.addr = address;
                host.port = port;
                host.active = active;
                let host = host.clone();
                if self.selected.remove(&original) {
                    self.selected.insert(name.clone());
                }
                if self.detail.as_deref() == Some(&original) {
                    self.detail = Some(name.clone());
                }
                if self.scope == original {
                    self.scope = name.clone();
                }
                if self.attack_filter == original {
                    self.attack_filter = name.clone();
                }
                self.status.remove(&original);
                self.update_clock.remove(&original);
                ctx.data_mut(|data| {
                    data.remove::<bool>(egui::Id::new(("server-address", &original)));
                    data.remove::<bool>(egui::Id::new(("server-address", &name)));
                });
                self.error.clear();
                if active {
                    self.start(&host);
                }
            }
            Err(err) => {
                self.error = format!("{err:#}");
                if let Some(host) = self.hosts.iter().find(|h| h.name == *original).cloned() {
                    let active = self
                        .db
                        .query_row(
                            "SELECT active FROM panel_hosts WHERE name=?",
                            [original],
                            |r| r.get::<_, bool>(0),
                        )
                        .unwrap_or(false);
                    if active {
                        self.start(&host);
                    }
                }
            }
        }
    }

    fn node_metrics(&mut self, ui: &mut Ui, host: &Host) {
        let address_id = egui::Id::new(("server-address", &host.name));
        let mut visible = ui.data_mut(|data| data.get_temp::<bool>(address_id).unwrap_or(false));
        let mut open = false;
        let mut edit = false;
        let status = self
            .status
            .get(&host.name)
            .map(String::as_str)
            .unwrap_or("Сбор остановлен");
        let color = if is_online(status) {
            glass::CYAN
        } else {
            MUTED
        };
        let address = if host.addr.contains(':') {
            format!("[{}]:{}", host.addr, host.port)
        } else {
            format!("{}:{}", host.addr, host.port)
        };
        glass::profile_row(ui, |ui| {
            let busy = status.starts_with("Обновление · ") || status.starts_with("Удаление · ");
            edit = ui
                .add_enabled_ui(!busy, |ui| {
                    glass::profile_icon(ui, true, false, "Изменить профиль").clicked()
                })
                .inner;
            if glass::profile_icon(
                ui,
                false,
                visible,
                if visible {
                    "Скрыть адрес"
                } else {
                    "Показать адрес"
                },
            )
            .clicked()
            {
                visible = !visible;
                ui.data_mut(|data| data.insert_temp(address_id, visible));
            }
            ui.vertical(|ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label(T::new("●").color(color).size(11.));
                    open = ui
                        .add(
                            egui::Label::new(T::new(&host.name).strong().size(19.))
                                .sense(egui::Sense::click()),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text("Открыть статистику")
                        .clicked();
                    if visible {
                        ui.label(T::new(&address).monospace().color(glass::ACCENT));
                    }
                    if !status.starts_with("Обновление · ") {
                        let shown = if visible {
                            status.to_owned()
                        } else {
                            status.replace(&host.addr, "скрытый адрес")
                        };
                        ui.label(T::new(shown).color(color));
                    }
                    if let Some((_, progress)) = status.rsplit_once(" · ") {
                        if let Some(percent) = progress
                            .strip_suffix('%')
                            .and_then(|n| n.parse::<f32>().ok())
                        {
                            if percent < 100. {
                                ui.add(
                                    egui::ProgressBar::new(percent / 100.)
                                        .desired_width(56.)
                                        .desired_height(4.)
                                        .fill(glass::CYAN),
                                );
                            }
                        }
                    }
                });
            });
        });
        if let Some(stage) = status.strip_prefix("Обновление · ") {
            Frame::new()
                .fill(C::from_rgb(40, 52, 55))
                .corner_radius(8)
                .inner_margin(8)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        ui.spinner();
                        ui.label(T::new(stage).strong().color(glass::ACCENT));
                        if let Some((started, signal)) = self.update_clock.get(&host.name) {
                            hint(ui, &format!("Этап {} с", started.elapsed().as_secs()));
                            if stage != "Ждём первый ответ сервера" {
                                let age = signal.elapsed().as_secs();
                                if age > 15 {
                                    ui.colored_label(C::LIGHT_RED, format!("Нет сигнала {age} с"));
                                } else {
                                    hint(ui, &format!("Сигнал {age} с назад"));
                                }
                            }
                        }
                    });
                });
        }
        ui.add_space(2.);
        let (packets, bytes, v4, v6, day_packets, day_bytes, before, recorded): (
            Option<i64>, Option<i64>, Option<i64>, Option<i64>, Option<i64>, Option<i64>,
            bool, i64,
        ) = self
            .db
            .query_row(
                "WITH first_sample AS (
                     SELECT coalesce(min(time),unixepoch()) AS start FROM samples WHERE host=?1
                 ), latest AS (
                     SELECT packets,bytes,family FROM samples WHERE rowid IN (
                         SELECT max(rowid) FROM samples WHERE host=?1 AND time=(
                             SELECT time FROM samples WHERE host=?1 ORDER BY rowid DESC LIMIT 1
                         ) GROUP BY family
                     )
                 )
                 SELECT sum(packets),sum(bytes),
                     sum(CASE WHEN family='4' THEN packets END),
                     sum(CASE WHEN family='6' THEN packets END),
                     (SELECT sum(dropped_packets) FROM samples WHERE host=?1 AND time>=strftime('%s','now')-86400),
                     (SELECT sum(dropped_bytes) FROM samples WHERE host=?1 AND time>=unixepoch()-86400),
                     (SELECT start>unixepoch()-86400 FROM first_sample),
                     (SELECT count(*) FROM events WHERE host=?1 AND time>=unixepoch()-86400
                         AND time<(SELECT start FROM first_sample))
                 FROM latest",
                [&host.name],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)),
            )
            .unwrap_or_default();
        let count = |n: Option<i64>| n.map_or("—".into(), |n| n.to_string());
        ui.horizontal_wrapped(|ui| {
            ui.label("Отброшено по счётчикам")
                .on_hover_text("Включая пакеты до привязки, пока счётчики файрволла не сброшены");
            ui.strong(format!("{} пакетов", count(packets)));
            ui.label(bytes.map_or("—".into(), |n| data_size(n as f64)));
            ui.label(T::new(format!("IPv4 {}   IPv6 {}", count(v4), count(v6))).color(MUTED));
        });
        ui.horizontal_wrapped(|ui| {
            hint(ui, "За 24 ч");
            let measured = match (day_packets, day_bytes) {
                (Some(packets), Some(bytes)) => format!("По замерам {packets} пакетов, {}", data_size(bytes as f64)),
                _ => "Пока нет замеров прироста".into(),
            };
            ui.label(measured);
            if before {
                let archive = if recorded > 0 {
                    format!("До первого замера по журналу: {recorded}")
                } else {
                    "До первого замера нет записей в журнале".into()
                };
                ui.label(T::new(archive).color(if recorded > 0 { glass::CYAN } else { MUTED }))
                    .on_hover_text("В журнал попадает часть отброшенных пакетов. Точный объём до первого замера неизвестен.");
            }
        });
        if let Ok(Some(m)) = db::metrics(&self.db, &host.name) {
            let sys = m.system;
            let ratio = |used: u64, total: u64| (total > 0).then(|| used as f64 / total as f64);
            ui.horizontal_wrapped(|ui| {
                meter(
                    ui,
                    "CPU",
                    m.cpu,
                    sys.cores.map_or("".into(), |n| format!("{n} ядер")),
                );
                meter(
                    ui,
                    "RAM",
                    ratio(sys.mem_used, sys.mem_total),
                    data_size(sys.mem_total as f64),
                );
                meter(
                    ui,
                    "Диск /",
                    ratio(sys.disk_used, sys.disk_total),
                    data_size(sys.disk_total as f64),
                );
            });
            let speed =
                |rate: Option<f64>| rate.map_or("—".into(), |n| format!("{}/с", data_size(n)));
            ui.horizontal_wrapped(|ui| {
                if let Some(interface) = sys.interface {
                    hint(
                        ui,
                        &format!(
                            "{interface}  ↓ {}  ↑ {}",
                            speed(m.rx_rate),
                            speed(m.tx_rate)
                        ),
                    );
                }
                let hours = sys.uptime as u64 / 3600;
                let minutes = sys.uptime as u64 / 60 % 60;
                hint(ui, &format!("Аптайм {hours} ч {minutes} мин"));
                let [one, five, fifteen] = sys.load;
                hint(ui, &format!("Load {one:.2} / {five:.2} / {fifteen:.2}"));
            });
        } else {
            hint(
                ui,
                "Ресурсы появятся после опроса сервера с новой версией rkn-guard.",
            );
        }
        if let Ok((version, firewall, last, branch, latest)) = self.db.query_row(
            "SELECT s.version,s.firewall,(SELECT max(time) FROM polls WHERE host=? AND status='ok'),
             v.branch,v.latest FROM panel_state s LEFT JOIN panel_versions v ON v.host=s.host
             WHERE s.host=?",
            params![host.name, host.name],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<i64>>(2)?,
                r.get::<_, Option<String>>(3)?, r.get::<_, Option<String>>(4)?)),
        ) {
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                ui.label(T::new(format!("rkn-guard {version}")).monospace().color(glass::ACCENT));
                if let Some(branch) = branch { hint(ui, &branch); }
                let (label, tint) = match latest {
                    Some(remote) if version.len() >= 7 && remote.starts_with(&version) =>
                        ("Актуальная".into(), glass::CYAN),
                    Some(remote) => (format!("Доступна {}", remote.get(..7).unwrap_or(&remote)), C::from_rgb(229, 184, 111)),
                    None => ("Версия не сверена".into(), MUTED),
                };
                Frame::new().fill(tint.gamma_multiply(0.12)).corner_radius(7)
                    .inner_margin(egui::Margin::symmetric(8, 3)).show(ui, |ui| {
                        ui.label(T::new(label).size(12.).color(tint));
                    });
                hint(ui, &firewall);
                hint(ui, &format!("Замер {}", last.map_or("—".into(), when)));
            });
        }
        if open {
            self.detail = Some(host.name.clone());
            self.remove.clear();
        }
        if edit {
            self.open_profile(host);
        }
    }

    fn send_actions(&mut self, names: &[String], action: Action, pending: &str) {
        for name in names {
            if let Some(worker) = self.workers.get(name) {
                if worker.tx.send(action.clone()).is_ok() {
                    self.status.insert(name.clone(), pending.into());
                } else {
                    self.status.insert(name.clone(), "Сбор остановлен".into());
                }
            }
        }
    }

    fn save_order(&mut self) {
        let order = self.hosts.iter().map(|h| &h.name).collect::<Vec<_>>();
        if let Err(err) = self.db.execute(
            "INSERT OR REPLACE INTO panel_settings VALUES('server_order',?)",
            [serde_json::to_string(&order).expect("имена серверов")],
        ) {
            self.error = err.to_string();
        }
    }

    fn server_toolbar(&mut self, ui: &mut Ui) {
        let selected: Vec<_> = self
            .hosts
            .iter()
            .filter(|h| self.selected.contains(&h.name))
            .cloned()
            .collect();
        let running: Vec<_> = selected
            .iter()
            .filter(|h| {
                self.workers.contains_key(&h.name)
                    && !self
                        .status
                        .get(&h.name)
                        .is_some_and(|status| status.starts_with("Обновление · "))
            })
            .map(|h| h.name.clone())
            .collect();
        let mut all = selected.len() == self.hosts.len();
        ui.horizontal_wrapped(|ui| {
            if ui.checkbox(&mut all, "Все").changed() {
                self.selected = if all {
                    self.hosts.iter().map(|h| h.name.clone()).collect()
                } else {
                    HashSet::new()
                };
                self.remove.clear();
            }
            if selected.is_empty() {
                hint(ui, "Отметьте серверы для групповых действий");
                return;
            }
            hint(ui, &format!("Выбрано {}", self.selected.len()));
            let ready = !running.is_empty() && self.remove.is_empty();
            if ui
                .add_enabled(ready, egui::Button::new("Опросить"))
                .clicked()
            {
                self.send_actions(&running, Action::Poll, "Опрос · 0%");
            }
            if ui
                .add_enabled(ready, egui::Button::new("Обновить"))
                .clicked()
            {
                self.send_actions(&running, Action::Update, "Обновление · В очереди");
            }
            if ui
                .add_enabled(
                    ready,
                    egui::Button::new(T::new("Удалить rkn-guard").color(C::LIGHT_RED)),
                )
                .clicked()
            {
                self.remove = running.clone();
            }
            if ui
                .add_enabled(
                    !selected.is_empty(),
                    egui::Button::new(
                        if selected.iter().any(|h| !self.workers.contains_key(&h.name)) {
                            "Запустить сбор"
                        } else {
                            "Остановить сбор"
                        },
                    ),
                )
                .clicked()
            {
                let start = selected.iter().any(|h| !self.workers.contains_key(&h.name));
                for host in selected
                    .iter()
                    .filter(|h| self.workers.contains_key(&h.name) != start)
                    .cloned()
                    .collect::<Vec<_>>()
                {
                    match self.db.execute(
                        "UPDATE panel_hosts SET active=? WHERE name=?",
                        params![start, host.name],
                    ) {
                        Ok(_) if start => self.start(&host),
                        Ok(_) => self.send_actions(&[host.name], Action::Stop, "Остановка…"),
                        Err(e) => self.error = e.to_string(),
                    }
                }
            }
            if let [host] = selected.as_slice() {
                if !self.workers.contains_key(&host.name) {
                    if ui.button("Убрать профиль").clicked() {
                        match self
                            .db
                            .execute("DELETE FROM panel_hosts WHERE name=?", [&host.name])
                        {
                            Ok(_) => {
                                self.hosts.retain(|h| h.name != host.name);
                                self.selected.remove(&host.name);
                                self.save_order();
                            }
                            Err(e) => self.error = e.to_string(),
                        }
                    }
                }
            }
        });
        if !self.remove.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(
                    C::LIGHT_RED,
                    format!("Удалить rkn-guard с серверов: {}?", self.remove.join(", ")),
                );
                if ui.button("Подтвердить удаление").clicked() {
                    let names = std::mem::take(&mut self.remove);
                    self.send_actions(&names, Action::Remove, "Удаление · 0%");
                } else if ui.button("Отмена").clicked() {
                    self.remove.clear();
                }
            });
        }
        ui.separator();
    }

    fn servers(&mut self, ui: &mut Ui) {
        if self.hosts.is_empty() {
            self.empty(ui);
            return;
        }
        let pointer = ui.input(|i| i.pointer.interact_pos());
        let released = ui.input(|i| i.pointer.any_released());
        let mut target = None;
        let hosts = self.hosts.clone();
        for (index, h) in hosts.iter().enumerate() {
            let active = self.selected.contains(&h.name);
            let row = card()
                .inner_margin(12)
                .fill(if active {
                    C::from_rgb(44, 57, 60)
                } else {
                    CARD
                })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let mut checked = active;
                        if ui
                            .checkbox(&mut checked, "")
                            .on_hover_text("Выбрать сервер")
                            .changed()
                        {
                            if checked {
                                self.selected.insert(h.name.clone());
                            } else {
                                self.selected.remove(&h.name);
                            }
                            self.remove.clear();
                        }
                        glass::profile_row(ui, |ui| {
                            let (rect, grip) =
                                ui.allocate_exact_size(egui::vec2(22., 42.), egui::Sense::drag());
                            let grip = grip
                                .on_hover_cursor(egui::CursorIcon::Grab)
                                .on_hover_text("Переместить сервер");
                            if grip.drag_started() {
                                self.dragging = Some(h.name.clone());
                            }
                            for x in [-2., 2.] {
                                for y in [-5., 0., 5.] {
                                    ui.painter().circle_filled(
                                        rect.center() + egui::vec2(x, y),
                                        1.,
                                        MUTED,
                                    );
                                }
                            }
                            ui.vertical(|ui| {
                                ui.set_min_width(ui.available_width());
                                self.node_metrics(ui, h);
                            });
                        });
                    });
                });
            if self.dragging.is_some()
                && pointer.is_some_and(|p| row.response.rect.intersect(ui.clip_rect()).contains(p))
            {
                target = Some(index);
                ui.painter().line_segment(
                    [row.response.rect.left_top(), row.response.rect.right_top()],
                    (2., glass::ACCENT),
                );
            }
        }
        if released {
            if let Some(name) = self.dragging.take() {
                if let (Some(from), Some(to)) =
                    (self.hosts.iter().position(|h| h.name == name), target)
                {
                    if from != to {
                        let host = self.hosts.remove(from);
                        self.hosts.insert(to, host);
                        self.save_order();
                    }
                }
            }
        }
    }
    fn empty(&mut self, ui: &mut Ui) {
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add_space(12.);
            ui.heading("Подключите первый сервер");
            hint(
                ui,
                "Получите код в меню rkn на сервере и задайте имя подключения.",
            );
            ui.add_space(6.);
            if main_button(ui, "Добавить по коду") {
                self.pair_open = true
            }
            ui.add_space(10.);
        });
    }
    fn overview(&mut self, ui: &mut Ui) {
        let (packets,bytes):(Option<i64>,Option<i64>)=self.db.query_row("SELECT sum(dropped_packets),sum(dropped_bytes) FROM samples WHERE time>=strftime('%s','now')-86400",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap_or_default();
        let stats = [
            ("Серверы", self.hosts.len().to_string()),
            (
                "Отброшено за 24 ч",
                packets.map(|x| x.to_string()).unwrap_or("—".into()),
            ),
            (
                "Объём за 24 ч",
                bytes
                    .map(|x| format!("{:.1} МиБ", x as f64 / 1048576.))
                    .unwrap_or("—".into()),
            ),
        ];
        ui.columns(3, |cols| {
            for (index, (col, (label, value))) in cols.iter_mut().zip(stats).enumerate() {
                card().show(col, |u| {
                    u.set_min_width((u.available_width()).max(60.));
                    let tint = [glass::ACCENT, glass::BLUE, glass::CYAN][index];
                    u.horizontal(|u| {
                        let (r, _) =
                            u.allocate_exact_size(egui::vec2(30., 30.), egui::Sense::hover());
                        u.painter().rect_filled(r, 8., tint.gamma_multiply(0.22));
                        glass::glyph(u.painter(), r.center(), [1, 0, 3][index], tint);
                        hint(u, label);
                    });
                    u.label(T::new(value).size(31.).strong());
                });
            }
        });
        ui.add_space(16.);
        if self.hosts.is_empty() {
            self.empty(ui)
        } else {
            ui.heading("Состояние серверов");
            for h in self.hosts.clone() {
                card().inner_margin(14).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    self.node_metrics(ui, &h);
                });
            }
        }
    }
    fn journal(&mut self, ui: &mut Ui) -> Result<()> {
        let mut stmt = self.db.prepare(
            "SELECT name FROM panel_hosts UNION SELECT host FROM events ORDER BY 1 COLLATE NOCASE",
        )?;
        let names = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let previous = self.scope.clone();
        let mut server = |ui: &mut Ui| {
            ui.label("Сервер");
            egui::ComboBox::from_id_salt("journal-server")
                .width(ui.available_width())
                .selected_text(if self.scope.is_empty() {
                    "Все серверы"
                } else {
                    &self.scope
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.scope, String::new(), "Все серверы");
                    for name in &names {
                        ui.selectable_value(&mut self.scope, name.clone(), name);
                    }
                });
        };
        let mut packet = |ui: &mut Ui| {
            ui.label("IP, протокол или порт");
            if field(ui, &mut self.filter, "Например, TCP или 443", false).changed() {
                self.offset = 0;
            }
        };
        if ui.available_width() >= 480. {
            ui.columns(2, |cols| {
                server(&mut cols[0]);
                packet(&mut cols[1]);
            });
        } else {
            server(ui);
            packet(ui);
        }
        if previous != self.scope {
            self.offset = 0;
        }
        if self.journal_dates.show(ui) {
            self.offset = 0;
        }
        let (since, until) = self.journal_dates.bounds;
        ui.add_space(8.);
        ui.separator();
        let mut q = self.db.prepare(
            "SELECT time,host,source,protocol,port,raw FROM events
             WHERE (?1='' OR host=?1)
               AND (?2='' OR instr(source,?2)>0 OR lower(protocol)=lower(?2) OR CAST(port AS TEXT)=?2)
               AND time>=?4 AND time<?5 ORDER BY time DESC,rowid DESC LIMIT 51 OFFSET ?3",
        )?;
        let rows = q
            .query_map(
                params![
                    self.scope,
                    self.filter,
                    self.offset,
                    since.unwrap_or(i64::MIN),
                    until.unwrap_or(i64::MAX)
                ],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, Option<i64>>(4)?,
                        r.get::<_, Option<String>>(5)?,
                    ))
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let importing: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM panel_log_archive a
             JOIN panel_hosts h ON h.name=a.host
             WHERE a.cursor IS NOT NULL AND (?1='' OR a.host=?1))",
            [&self.scope],
            |r| r.get(0),
        )?;
        if importing {
            ui.horizontal(|ui| {
                ui.spinner();
                hint(ui, "Загружаются сохранённые журналы…");
            });
        }
        if rows.is_empty() && !importing {
            hint(
                ui,
                if self.scope.is_empty()
                    && self.filter.is_empty()
                    && since.is_none()
                    && until.is_none()
                {
                    "В журнале пока нет пакетов."
                } else {
                    "Пакеты не найдены. Попробуйте изменить фильтр."
                },
            );
        }
        for (i, (t, h, ip, proto, port, raw)) in rows.iter().take(50).enumerate() {
            ui.push_id((self.offset, i), |ui| {
                ui.collapsing(
                    format!(
                        "{}     {}     {}     {}   {}",
                        when(*t),
                        h,
                        ip,
                        proto,
                        port.map(|v| v.to_string()).unwrap_or_default()
                    ),
                    |ui| {
                        ui.monospace(raw.as_deref().unwrap_or("—"));
                    },
                );
            });
        }
        if self.offset == 0 && rows.len() <= 50 {
            return Ok(());
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.offset > 0, egui::Button::new("Новее"))
                .clicked()
            {
                self.offset = (self.offset - 50).max(0);
            }
            if ui
                .add_enabled(rows.len() > 50, egui::Button::new("Старее"))
                .clicked()
            {
                self.offset += 50;
            }
        });
        Ok(())
    }
    fn analytics(&mut self, ui: &mut Ui) -> Result<()> {
        let now = chrono::Utc::now().timestamp() / 3600;
        let mut bins = [None; 24];
        let mut q=self.db.prepare("SELECT time/3600,sum(dropped_packets) FROM samples WHERE time/3600 BETWEEN ? AND ? GROUP BY time/3600")?;
        for row in q.query_map(params![now - 23, now], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?))
        })? {
            let (hour, n) = row?;
            bins[(hour - now + 23) as usize] = n;
        }
        card().show(ui, |ui| {
            ui.heading("Отброшенные пакеты");
            hint(ui, "По часам за последние 24 часа");
            let peak = bins.iter().flatten().copied().max().unwrap_or(0);
            let max = peak.max(1);
            let (r, _) = ui
                .allocate_exact_size(egui::vec2(ui.available_width(), 154.), egui::Sense::hover());
            let p = ui.painter();
            let plot = r.shrink2(egui::vec2(4., 16.));
            for j in 0..4 {
                let y = plot.bottom() - j as f32 * plot.height() / 3.;
                p.line_segment(
                    [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
                    (1., LINE),
                );
            }
            for (i, v) in bins.iter().enumerate() {
                if let Some(n) = v {
                    let w = plot.width() / 24.;
                    let x = plot.left() + i as f32 * w;
                    p.rect_filled(
                        egui::Rect::from_min_max(
                            egui::pos2(
                                x,
                                plot.bottom()
                                    - ((*n as f64 / max as f64) * (plot.height() - 1.) as f64)
                                        as f32
                                    - 1.,
                            ),
                            egui::pos2(x + w * 0.72, plot.bottom()),
                        ),
                        2.,
                        glass::CYAN,
                    );
                }
            }
            for i in [0, 6, 12, 18, 23] {
                let stamp = chrono::DateTime::from_timestamp((now - 23 + i) * 3600, 0)
                    .unwrap()
                    .with_timezone(&chrono::Local);
                let x = egui::lerp(plot.x_range(), i as f32 / 23.);
                let anchor = match i {
                    0 => egui::Align2::LEFT_BOTTOM,
                    23 => egui::Align2::RIGHT_BOTTOM,
                    _ => egui::Align2::CENTER_BOTTOM,
                };
                p.text(
                    egui::pos2(x, r.bottom() + 3.),
                    anchor,
                    stamp.format("%H:%M"),
                    egui::FontId::monospace(11.),
                    MUTED,
                );
            }
            if bins.iter().all(Option::is_none) {
                let tag = egui::Rect::from_center_size(plot.center(), egui::vec2(170., 33.));
                p.rect_filled(tag, 16., SIDE);
                p.rect_stroke(tag, 16., (1., LINE), egui::StrokeKind::Inside);
                p.text(
                    tag.center(),
                    egui::Align2::CENTER_CENTER,
                    "Ожидание замеров",
                    egui::FontId::proportional(14.),
                    MUTED,
                );
            } else {
                hint(ui, &format!("Максимум за час: {peak}"))
            }
        });
        drop(q);
        self.attack_table(ui, None)
    }

    fn attack_table(&mut self, ui: &mut Ui, server: Option<&str>) -> Result<()> {
        if !self.hosts.iter().any(|h| h.name == self.attack_filter) {
            self.attack_filter.clear();
        }
        card().show(ui, |ui| -> Result<()> {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.heading("Топ атак");
                if server.is_none() {
                    egui::ComboBox::from_id_salt("attack-server")
                        .selected_text(if self.attack_filter.is_empty() { "Все серверы" } else { &self.attack_filter })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.attack_filter, String::new(), "Все серверы");
                            for host in &self.hosts {
                                ui.selectable_value(&mut self.attack_filter, host.name.clone(), &host.name);
                            }
                        });
                }
            });
            let filter = server.unwrap_or(&self.attack_filter);
            let mut query = self.db.prepare(
                "SELECT a.host,a.ip,a.asn,a.network,a.count,a.last
                 FROM panel_attack_sources a JOIN panel_hosts h ON h.name=a.host
                 WHERE (?1='' OR a.host=?1)
                 ORDER BY a.count DESC,a.host,a.ip LIMIT 100",
            )?;
            let mut rows = query.query([filter])?;
            if rows.next()?.is_none() {
                hint(ui, "Пока нет сводки сборщика. Обновите rkn-guard на сервере и дождитесь опроса.");
                return Ok(());
            }
            hint(ui, "За всё время, по данным серверного сборщика");
            drop(rows);
            let mut rows = query.query([filter])?;
            egui::ScrollArea::both().id_salt("attack-table").max_height(390.).show(ui, |ui| -> Result<()> {
                egui::Grid::new("attack-sources").striped(true).spacing([18., 8.]).show(ui, |ui| -> Result<()> {
                    if server.is_none() { ui.strong("Сервер"); }
                    for label in ["IP", "ASN", "Сеть", "События", "Последнее событие"] {
                        ui.strong(label);
                    }
                    ui.end_row();
                    while let Some(row) = rows.next()? {
                        if server.is_none() { ui.label(row.get::<_, String>(0)?); }
                        ui.monospace(row.get::<_, String>(1)?);
                        hint(ui, &row.get::<_, String>(2)?);
                        ui.label(row.get::<_, String>(3)?);
                        ui.strong(row.get::<_, i64>(4)?.to_string());
                        let raw: String = row.get(5)?;
                        let stamp = chrono::DateTime::parse_from_rfc3339(&raw).map(|t| when(t.timestamp())).unwrap_or(raw);
                        hint(ui, &stamp);
                        ui.end_row();
                    }
                    Ok(())
                }).inner
            }).inner
        }).inner?;
        Ok(())
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.dispatch();
        self.finish_profile_edit(ctx);
        ctx.request_repaint_after(Duration::from_secs(1));
        egui::SidePanel::left("rail")
            .resizable(false)
            .exact_width(213.)
            .frame(Frame::new().fill(SIDE).inner_margin(17))
            .show(ctx, |ui| {
                egui::TopBottomPanel::bottom("rail-footer")
                    .exact_height(132.)
                    .frame(Frame::new())
                    .show_inside(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 5.;
                        ui.add_space(6.);
                        let online = self
                            .hosts
                            .iter()
                            .filter(|h| self.status.get(&h.name).is_some_and(|s| is_online(s)))
                            .count();
                        ui.horizontal(|ui| {
                            let (r, _) =
                                ui.allocate_exact_size(egui::vec2(10., 14.), egui::Sense::hover());
                            ui.painter().circle_filled(
                                r.center(),
                                3.,
                                if online > 0 { glass::CYAN } else { MUTED },
                            );
                            hint(ui, &format!("На связи  {online} / {}", self.hosts.len()));
                        });
                        ui.add_space(5.);
                        let busy = self.pair_result.is_some()
                            || self.saving_profile
                            || self.status.values().any(|status| {
                                status.starts_with("Обновление · ")
                                    || status.starts_with("Удаление · ")
                            });
                        self.updater.show(ui, busy);
                    });
                egui::ScrollArea::vertical()
                    .id_salt("rail-navigation")
                    .show(ui, |ui| {
                        ui.add_space(8.);
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::Image::new((self.logo.id(), egui::vec2(36., 35.)))
                                    .corner_radius(9.),
                            );
                            ui.label(T::new(crate::APP_NAME).strong().size(17.).color(MINT));
                        });
                        ui.add_space(18.);
                        for (index, (p, label)) in [
                            (Page::Overview, "Мониторинг"),
                            (Page::Servers, "Серверы"),
                            (Page::Journal, "Журнал пакетов"),
                            (Page::Analytics, "Аналитика"),
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            if glass::nav(ui, label, index, self.page == p) {
                                self.detail = None;
                                self.remove.clear();
                                self.page = p;
                            }
                        }
                    });
            });
        egui::CentralPanel::default()
            .frame(Frame::new().fill(BG).inner_margin(24))
            .show(ctx, |ui| {
                if let Some(name) = self.detail.clone() {
                    self.server_detail(ui, &name);
                    return;
                }
                ui.horizontal_wrapped(|ui| {
                    title(
                        ui,
                        match self.page {
                            Page::Overview => "Мониторинг",
                            Page::Servers => "Серверы",
                            Page::Journal => "Журнал пакетов",
                            Page::Analytics => "Аналитика",
                        },
                    );
                    if !self.hosts.is_empty()
                        && self.page == Page::Servers
                        && main_button(ui, "Добавить по коду")
                    {
                        self.pair_open = true
                    }
                    if self.page == Page::Servers
                        && !self.hosts.is_empty()
                        && ui.button("Управление").clicked()
                    {
                        self.management.open = !self.management.open;
                        self.management.catalogs.retain(|_, state| state.is_none());
                        self.management.rules.retain(|_, state| state.is_none());
                    }
                });
                ui.add_space(14.);
                if !self.error.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(C::LIGHT_RED, &self.error);
                        if ui.button("Закрыть").clicked() {
                            self.error.clear()
                        }
                    });
                }
                if self.page == Page::Servers && !self.hosts.is_empty() {
                    self.server_toolbar(ui);
                }
                egui::ScrollArea::vertical()
                    .id_salt(self.page)
                    .show(ui, |ui| {
                        let result = match self.page {
                            Page::Overview => {
                                self.overview(ui);
                                ui.add_space(12.);
                                self.analytics(ui)
                            }
                            Page::Servers => {
                                self.servers(ui);
                                Ok(())
                            }
                            Page::Journal => self.journal(ui),
                            Page::Analytics => self.analytics(ui),
                        };
                        if let Err(e) = result {
                            self.error = e.to_string();
                        }
                    });
            });
        if self.page == Page::Servers && self.detail.is_none() && self.management.open {
            let mut open = true;
            let width = (ctx.screen_rect().width() * 0.43).clamp(330., 480.);
            egui::Window::new("Управление серверами")
                .id(egui::Id::new("management-drawer"))
                .anchor(egui::Align2::RIGHT_TOP, [-8., 8.])
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .fixed_size([width, (ctx.screen_rect().height() - 48.).max(200.)])
                .vscroll(true)
                .show(ctx, |ui| {
                    self.management
                        .show(ui, &self.hosts, &self.selected, &self.workers)
                });
            self.management.open = open;
        }
        if let Some((original, mut name, mut address, mut port)) = self.edit.take() {
            let mut keep = true;
            egui::Modal::new(egui::Id::new("edit-profile"))
                .frame(card().inner_margin(22))
                .show(ctx, |ui| {
                    ui.set_width(350.);
                    ui.heading("Изменить профиль");
                    ui.add_enabled_ui(!self.saving_profile, |ui| {
                        ui.label("Имя сервера");
                        field(ui, &mut name, "Например, Москва-01", false);
                        ui.label("IP или DNS-имя");
                        field(ui, &mut address, "Адрес сервера", false);
                        ui.label("SSH-порт");
                        ui.add(egui::DragValue::new(&mut port).range(1..=65535));
                        ui.horizontal(|ui| {
                            if main_button(ui, "Сохранить") {
                                name = name.trim().to_owned();
                                address = address
                                    .trim()
                                    .trim_start_matches('[')
                                    .trim_end_matches(']')
                                    .to_owned();
                                if name.is_empty()
                                    || name.chars().count() > 80
                                    || name.chars().any(char::is_control)
                                    || address.is_empty()
                                    || address.len() > 253
                                    || address.starts_with('-')
                                    || !address
                                        .bytes()
                                        .all(|b| b.is_ascii_alphanumeric() || b".:-".contains(&b))
                                    || (address.contains(':')
                                        && address.parse::<std::net::Ipv6Addr>().is_err())
                                {
                                    self.error =
                                        "Введите имя и корректный IP или DNS-адрес.".into();
                                } else {
                                    self.error.clear();
                                    self.saving_profile = true;
                                    if let Some(worker) = self.workers.get(&original) {
                                        let _ = worker.tx.send(Action::Stop);
                                        self.status
                                            .insert(original.clone(), "Переподключение…".into());
                                    }
                                }
                            }
                            if ui.button("Отмена").clicked() {
                                self.error.clear();
                                keep = false;
                            }
                        });
                    });
                    if self.saving_profile {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            hint(ui, "Сохраняем профиль и переподключаемся…");
                        });
                    } else if !self.error.is_empty() {
                        ui.colored_label(C::LIGHT_RED, &self.error);
                    }
                });
            if keep {
                self.edit = Some((original, name, address, port));
            }
        }
        if self.pair_open {
            egui::Modal::new(egui::Id::new("pair-server"))
                .frame(card().inner_margin(22))
                .show(ctx, |ui| {
                    ui.set_width((ctx.screen_rect().width() - 64.).clamp(160., 372.));
                    ui.heading("Добавить сервер");
                    ui.add_space(6.);
                    ui.label("Имя сервера");
                    field(ui, &mut self.name, "Например, Москва-01", false);
                    ui.label("Одноразовый код");
                    field(ui, &mut self.code, "Вставьте код из меню rkn", true);
                    ui.add_space(8.);
                    if self.pair_result.is_some() {
                        ui.spinner();
                    } else {
                        ui.horizontal(|ui| {
                            let ready =
                                !self.name.trim().is_empty() && !self.code.trim().is_empty();
                            if ui
                                .add_enabled_ui(ready, |ui| main_button(ui, "Подключить"))
                                .inner
                            {
                                let (tx, rx) = mpsc::channel();
                                self.pair_result = Some(rx);
                                let args =
                                    (self.path.clone(), self.name.clone(), self.code.clone());
                                thread::spawn(move || {
                                    let _ = tx.send(ssh::pair(&args.0, args.1, args.2));
                                });
                            }
                            if ui.button("Отмена").clicked() {
                                self.code.clear();
                                self.pair_open = false
                            }
                        });
                    }
                    if !self.error.is_empty() {
                        ui.colored_label(C::LIGHT_RED, &self.error);
                    }
                });
        }
    }
}
