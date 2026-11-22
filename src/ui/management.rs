use super::*;
use std::collections::BTreeMap;
use std::net::IpAddr;

struct Command {
    id: &'static str,
    label: &'static str,
    input: &'static str,
}

fn source_row(
    ui: &mut Ui,
    name: &str,
    url: &str,
    enabled: usize,
    known: usize,
    selected: bool,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 52.), egui::Sense::click());
    let p = ui.painter();
    let fill = if selected {
        C::from_rgb(43, 59, 62)
    } else if response.hovered() {
        SIDE
    } else {
        CARD
    };
    p.rect(
        rect,
        8.,
        fill,
        (1., if selected { glass::CYAN } else { LINE }),
        egui::StrokeKind::Inside,
    );
    let left = rect.left() + 12.;
    let badge = format!("{enabled} / {known}");
    p.text(
        egui::pos2(rect.right() - 12., rect.center().y),
        egui::Align2::RIGHT_CENTER,
        badge,
        egui::FontId::monospace(13.),
        if enabled > 0 { glass::CYAN } else { MUTED },
    );
    let text_rect = egui::Rect::from_min_max(
        egui::pos2(left, rect.top()),
        egui::pos2((rect.right() - 82.).max(left), rect.bottom()),
    );
    let p = p.with_clip_rect(ui.clip_rect().intersect(text_rect));
    let host = url
        .split_once("://")
        .map(|(_, rest)| rest.split('/').next().unwrap_or(rest))
        .unwrap_or(url);
    for (text, y, size, color) in [(name, 16., 15., C::WHITE), (host, 36., 12., MUTED)] {
        let mut job = egui::text::LayoutJob::simple_singleline(
            text.into(),
            egui::FontId::proportional(size),
            color,
        );
        job.wrap.max_width = text_rect.width();
        job.wrap.max_rows = 1;
        let galley = p.layout_job(job);
        p.galley(
            egui::pos2(left, rect.top() + y - galley.size().y / 2.),
            galley,
            color,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, name)
    });
    response.on_hover_text(format!(
        "{url}\nВключён на {enabled} из {known} опрошенных серверов"
    ))
}

const IP: &str = "IP-адрес";
const RULE: &str = "IP, протокол и порт";
const URL: &str = "URL списка";
const INTERVAL: &str = "Часы (1–168)";
const HOURS: &str = "Время МСК через запятую";
const FIREWALL: &str = "Файрвол";
const GROUPS: &[(&str, &[Command])] = &[
    (
        "Управление IP",
        &[
            cmd("ban-list", "Ручные баны", ""),
            cmd("ban", "Забанить IP", IP),
            cmd("unban", "Разбанить IP", IP),
            cmd("exception-list", "Исключения", ""),
            cmd("exception-add", "Добавить исключение", RULE),
            cmd("exception-delete", "Удалить исключение", RULE),
        ],
    ),
    (
        "Обновление подсетей",
        &[
            cmd("schedule-show", "Текущее расписание", ""),
            cmd("refresh", "Обновить подсети сейчас", ""),
            cmd("interval", "Обновлять с интервалом", INTERVAL),
            cmd("hours", "Обновлять по часам", HOURS),
            cmd("msk", "Ежедневно в 06:00 МСК", ""),
            cmd("local", "В 06:00 по времени сервера", ""),
            cmd("off", "Выключить расписание", ""),
        ],
    ),
    ("Источники подсетей", &[]),
    (
        "Переустановка",
        &[cmd("reinstall", "Переустановить rkn-guard", FIREWALL)],
    ),
];

const fn cmd(id: &'static str, label: &'static str, input: &'static str) -> Command {
    Command { id, label, input }
}

impl Command {
    fn valid(&self, value: &str) -> bool {
        match self.input {
            IP => value.parse::<IpAddr>().is_ok(),
            RULE => {
                let mut parts = value.split_whitespace();
                parts.next().is_some_and(|ip| ip.parse::<IpAddr>().is_ok())
                    && matches!(parts.next(), Some("tcp" | "udp"))
                    && parts
                        .next()
                        .is_some_and(|p| p.parse::<u16>().is_ok_and(|port| port > 0))
                    && parts.next().is_none()
            }
            URL => {
                value.len() <= 4096
                    && ["https://", "http://"]
                        .iter()
                        .any(|scheme| value.strip_prefix(scheme).is_some_and(|s| !s.is_empty()))
                    && !value.chars().any(char::is_whitespace)
            }
            INTERVAL => value.parse::<u16>().is_ok_and(|h| (1..=168).contains(&h)),
            HOURS => {
                !value.is_empty()
                    && value.len() <= 256
                    && value.split(',').count() <= 24
                    && value.split(',').all(|v| {
                        let v = v.trim();
                        let (hour, minute) = v.split_once(':').unwrap_or((v, "00"));
                        hour.len() <= 2
                            && minute.len() == 2
                            && hour.parse::<u8>().is_ok_and(|h| h < 24)
                            && minute.parse::<u8>().is_ok_and(|m| m < 60)
                    })
            }
            _ => true,
        }
    }
}

struct Report {
    label: &'static str,
    result: Option<Result<String, String>>,
}

pub(super) struct Management {
    pub open: bool,
    pub rules: BTreeMap<String, Option<Result<crate::catalog::Catalog, String>>>,
    exception: bool,
    selected_rule: Option<(bool, String)>,
    pub catalogs: BTreeMap<String, Option<Result<crate::catalog::Catalog, String>>>,
    source: Option<(bool, String)>,
    adding: bool,
    search: String,
    group: usize,
    action: usize,
    value: String,
    reports: BTreeMap<(String, usize), Report>,
}

impl Default for Management {
    fn default() -> Self {
        Self {
            open: false,
            rules: Default::default(),
            exception: false,
            selected_rule: None,
            catalogs: Default::default(),
            source: None,
            adding: false,
            search: String::new(),
            group: 2,
            action: 0,
            value: String::new(),
            reports: Default::default(),
        }
    }
}

impl Management {
    pub fn finish(&mut self, name: &str, result: Result<String, String>) {
        if let Some(((_, group), report)) = self
            .reports
            .iter_mut()
            .find(|((host, _), r)| host == name && r.result.is_none())
        {
            report.result = Some(result);
            if *group == 2 {
                self.catalogs.remove(name);
            }
            if *group == 0 {
                self.rules.remove(name);
            }
        }
    }
    pub fn disconnected(&mut self, name: &str) {
        self.rules.remove(name);
        self.catalogs.remove(name);
        if self.pending(name) {
            self.finish(
                name,
                Err("Соединение закрыто. Проверьте состояние сервера.".into()),
            );
        }
    }
    fn pending(&self, name: &str) -> bool {
        self.reports
            .iter()
            .any(|((host, _), r)| host == name && r.result.is_none())
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        hosts: &[Host],
        selected: &HashSet<String>,
        workers: &HashMap<String, Worker>,
    ) {
        ui.label(format!("Выбрано серверов: {}", selected.len()));
        ui.horizontal_wrapped(|ui| {
            for (i, label) in [
                (2, "Списки"),
                (1, "Расписание"),
                (0, "IP и баны"),
                (3, "Переустановка"),
            ] {
                if ui.selectable_label(self.group == i, label).clicked() && self.group != i {
                    self.group = i;
                    self.action = 0;
                    self.value.clear();
                }
            }
        });
        ui.separator();
        if self.group == 2 {
            self.sources(ui, hosts, selected, workers);
        } else if self.group == 0 {
            self.ip_rules(ui, hosts, selected, workers);
        } else {
            ui.horizontal_wrapped(|ui| {
                let cmds = GROUPS[self.group].1;
                let previous = self.action;
                egui::ComboBox::from_id_salt("management-action")
                    .selected_text(cmds[self.action].label)
                    .show_ui(ui, |ui| {
                        for (i, command) in cmds.iter().enumerate() {
                            ui.selectable_value(&mut self.action, i, command.label);
                        }
                    });
                if previous != self.action {
                    self.value.clear();
                }
            });
            let command = &GROUPS[self.group].1[self.action];
            match command.input {
                FIREWALL => {
                    if self.value.is_empty() {
                        self.value = "auto".into();
                    }
                    egui::ComboBox::from_label(FIREWALL)
                        .selected_text(&self.value)
                        .show_ui(ui, |ui| {
                            for backend in ["auto", "iptables", "nftables", "firewalld"] {
                                ui.selectable_value(&mut self.value, backend.into(), backend);
                            }
                        });
                    hint(
                        ui,
                        "Будут заново установлены rkn-guard и правила защиты. Расписание сохранится.",
                    );
                }
                "" => {}
                input => {
                    ui.label(input);
                    field(
                        ui,
                        &mut self.value,
                        match input {
                            RULE => "192.0.2.1 tcp 443",
                            HOURS => "06:00, 18:30",
                            INTERVAL => "6",
                            URL => "https://…",
                            _ => "IPv4 или IPv6",
                        },
                        false,
                    );
                }
            }
            let targets: Vec<_> = hosts
                .iter()
                .filter(|h| selected.contains(&h.name))
                .collect();
            let ready = !targets.is_empty()
                && targets
                    .iter()
                    .all(|h| workers.contains_key(&h.name) && !self.pending(&h.name));
            if !ready {
                hint(
                    ui,
                    if targets.is_empty() {
                        "Отметьте серверы в списке."
                    } else {
                        "Для запуска нужен включённый мониторинг и завершение предыдущей команды на каждом выбранном сервере."
                    },
                );
            }
            let value = if matches!(command.id, "msk" | "local") {
                "06:00"
            } else {
                self.value.trim()
            };
            if ui
                .add_enabled(
                    ready && command.valid(value),
                    egui::Button::new(format!("{} · серверов {}", command.label, targets.len())),
                )
                .clicked()
            {
                for h in targets {
                    let sent = workers[&h.name].tx.send(Action::Manage {
                        action: command.id,
                        value: value.into(),
                    });
                    self.reports.insert(
                        (h.name.clone(), self.group),
                        Report {
                            label: command.label,
                            result: sent.err().map(|_| Err("Сбор остановлен".into())),
                        },
                    );
                }
            }
        }
        for ((host, group), report) in self
            .reports
            .iter()
            .filter(|((name, group), _)| *group == self.group && selected.contains(name))
        {
            let state = match &report.result {
                None => "В очереди или выполняется",
                Some(Ok(_)) => "Готово",
                Some(Err(_)) => "Ошибка",
            };
            egui::CollapsingHeader::new(format!("{host} · {} · {state}", report.label))
                .id_salt(("command-result", host, group))
                .show(ui, |ui| {
                    if let Some(result) = &report.result {
                        let text = match result {
                            Ok(s) | Err(s) => s,
                        };
                        egui::ScrollArea::vertical()
                            .id_salt(host)
                            .max_height(180.)
                            .show(ui, |ui| {
                                ui.label(text);
                            });
                    }
                });
        }
    }

    fn sources(
        &mut self,
        ui: &mut Ui,
        hosts: &[Host],
        selected: &HashSet<String>,
        workers: &HashMap<String, Worker>,
    ) {
        self.catalogs
            .retain(|name, state| selected.contains(name) || state.is_none());
        let targets: Vec<_> = hosts
            .iter()
            .filter(|h| selected.contains(&h.name))
            .collect();
        if targets.is_empty() {
            hint(ui, "Отметьте серверы слева.");
            return;
        }
        ui.horizontal_wrapped(|ui| {
            if ui.button("Обновить состояния").clicked() {
                self.catalogs
                    .retain(|name, state| !selected.contains(name) || state.is_none());
            }
            if ui.selectable_label(self.adding, "Новый список").clicked() {
                self.adding = !self.adding;
                self.value.clear();
            }
        });
        let mut sources = std::collections::BTreeSet::new();
        for h in &targets {
            let name = &h.name;
            let Some(worker) = workers.get(name) else {
                hint(ui, &format!("{name} · сбор остановлен"));
                continue;
            };
            if !self.catalogs.contains_key(name) && !self.pending(name) {
                let error = worker
                    .tx
                    .send(Action::Catalog)
                    .err()
                    .map(|_| Err("Сбор остановлен".into()));
                self.catalogs.insert(name.clone(), error);
            }
            match self.catalogs.get(name) {
                Some(Some(Ok(entries))) => sources.extend(entries.keys().cloned()),
                Some(Some(Err(error))) => {
                    ui.colored_label(C::LIGHT_RED, format!("{name} · {error}"));
                }
                _ => hint(ui, &format!("{name} · загрузка состояния…")),
            }
        }
        if self.adding {
            ui.label("URL нового списка");
            field(ui, &mut self.value, "https://…", false);
            let url = self.value.trim().to_string();
            let names = self.absent_hosts(&targets, &url);
            self.source_button(ui, "add", "Добавить", names, &url, workers);
        }
        if !sources.is_empty() || !self.search.is_empty() {
            field(
                ui,
                &mut self.search,
                "Найти список по названию или адресу",
                false,
            );
        }
        let query = self.search.trim().to_lowercase();
        let mut found = false;
        for base in [true, false] {
            let rows: Vec<_> = sources
                .iter()
                .filter(|(b, url)| *b == base && url.to_lowercase().contains(&query))
                .collect();
            if rows.is_empty() {
                continue;
            }
            ui.add_space(8.);
            found = true;
            let group = if base {
                "Базовые списки"
            } else {
                "Дополнительные списки"
            };
            ui.label(
                T::new(format!("{group}  ·  {}", rows.len()))
                    .color(MUTED)
                    .small(),
            );
            for key in rows {
                let (_, url) = key;
                let enabled = targets
                    .iter()
                    .filter(|h| {
                        self.entries(&h.name).is_some_and(|entries| {
                            entries.iter().any(|((_, u), on)| u == url && *on)
                        })
                    })
                    .count();
                let label = url
                    .split('?')
                    .next()
                    .unwrap_or(url)
                    .trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .unwrap_or(url);
                let selected = self.source.as_ref() == Some(key);
                let known = targets
                    .iter()
                    .filter(|h| self.entries(&h.name).is_some())
                    .count();
                if source_row(ui, label, url, enabled, known, selected).clicked() {
                    self.source = if selected { None } else { Some(key.clone()) };
                    self.adding = false;
                }
                if self.source.as_ref() != Some(key) {
                    continue;
                }
                ui.add(
                    egui::Label::new(T::new(url).small().color(MUTED))
                        .wrap()
                        .selectable(true),
                );
                let mut on = Vec::new();
                let mut off = Vec::new();
                for h in &targets {
                    let state = match self.entries(&h.name) {
                        Some(entries) => match entries.get(key) {
                            Some(true) => {
                                on.push(h.name.clone());
                                "Включён"
                            }
                            Some(false) => {
                                off.push(h.name.clone());
                                "Выключен"
                            }
                            None => match entries.get(&(!base, url.clone())) {
                                Some(true) if base => "Включён как дополнительный",
                                Some(false) if base => "Выключен как дополнительный",
                                Some(true) => "Включён как базовый",
                                Some(false) => "Выключен как базовый",
                                None => "Нет списка",
                            },
                        },
                        None => "Нет данных",
                    };
                    ui.horizontal(|ui| {
                        ui.label(&h.name);
                        ui.colored_label(
                            if state.starts_with("Включён") {
                                MINT
                            } else {
                                MUTED
                            },
                            state,
                        );
                    });
                }
                ui.horizontal_wrapped(|ui| {
                    self.source_button(
                        ui,
                        if base { "base-enable" } else { "enable" },
                        "Включить",
                        off.clone(),
                        url,
                        workers,
                    );
                    self.source_button(
                        ui,
                        if base { "base-disable" } else { "disable" },
                        "Выключить",
                        on.clone(),
                        url,
                        workers,
                    );
                    if !base {
                        on.extend(off);
                        self.source_button(ui, "remove", "Удалить список", on, url, workers);
                    }
                    let absent = self.absent_hosts(&targets, url);
                    if !absent.is_empty() {
                        self.source_button(
                            ui,
                            "add",
                            "Добавить как дополнительный",
                            absent,
                            url,
                            workers,
                        );
                    }
                });
                ui.separator();
            }
        }
        if sources.is_empty() && targets.iter().all(|h| self.entries(&h.name).is_some()) {
            hint(ui, "На выбранных серверах списков нет.");
        }
        if !found && !sources.is_empty() {
            hint(ui, "Совпадений нет.");
            if ui.button("Очистить поиск").clicked() {
                self.search.clear();
            }
        }
    }

    fn ip_rules(
        &mut self,
        ui: &mut Ui,
        hosts: &[Host],
        selected: &HashSet<String>,
        workers: &HashMap<String, Worker>,
    ) {
        let targets: Vec<_> = hosts
            .iter()
            .filter(|h| selected.contains(&h.name))
            .collect();
        self.rules
            .retain(|n, v| selected.contains(n) || v.is_none());
        if targets.is_empty() {
            hint(ui, "Отметьте серверы слева.");
            return;
        }
        ui.horizontal_wrapped(|ui| {
            let previous = self.exception;
            ui.selectable_value(&mut self.exception, false, "Ручные баны");
            ui.selectable_value(&mut self.exception, true, "Исключения");
            if previous != self.exception {
                self.value.clear();
            }
            if ui.button("Обновить").clicked() {
                self.rules.retain(|_, v| v.is_none());
            }
        });
        let mut union = std::collections::BTreeSet::new();
        let mut known = Vec::new();
        for h in &targets {
            let n = &h.name;
            let Some(w) = workers.get(n) else {
                hint(ui, &format!("{n} · сбор остановлен"));
                continue;
            };
            if !self.rules.contains_key(n) && !self.pending(n) {
                let error =
                    w.tx.send(Action::Rules)
                        .err()
                        .map(|_| Err("Сбор остановлен".into()));
                self.rules.insert(n.clone(), error);
            }
            match self.rules.get(n) {
                Some(Some(Ok(rows))) => {
                    known.push(n.clone());
                    union.extend(rows.keys().filter(|(ex, _)| *ex == self.exception).cloned());
                }
                Some(Some(Err(e))) => {
                    ui.colored_label(C::LIGHT_RED, format!("{n} · {e}"));
                }
                _ => hint(ui, &format!("{n} · загрузка…")),
            }
        }
        let (add, remove, add_label, remove_label, kind) = if self.exception {
            (
                "exception-add",
                "exception-delete",
                "Добавить исключение",
                "Удалить исключение",
                RULE,
            )
        } else {
            ("ban", "unban", "Забанить", "Разбанить", IP)
        };
        field(
            ui,
            &mut self.value,
            if self.exception {
                "192.0.2.1 tcp 443"
            } else {
                "IPv4 или IPv6"
            },
            false,
        );
        let input = self.value.trim().to_string();
        let present = |this: &Self, n: &String, key: &(bool, String)| {
            this.rules[n]
                .as_ref()
                .and_then(|r| r.as_ref().ok())
                .is_some_and(|rows| rows.contains_key(key))
        };
        let key = (self.exception, input.clone());
        let absent = known
            .iter()
            .filter(|n| !present(self, n, &key))
            .cloned()
            .collect();
        self.action_button(ui, add, add_label, absent, &input, workers, kind);
        if union.is_empty() && known.len() == targets.len() {
            hint(
                ui,
                if self.exception {
                    "Исключений нет."
                } else {
                    "Ручных банов нет."
                },
            );
        }
        for key in union {
            let applied: Vec<_> = known
                .iter()
                .filter(|n| present(self, n, &key))
                .cloned()
                .collect();
            if ui
                .selectable_label(
                    self.selected_rule.as_ref() == Some(&key),
                    format!("{}   ·   {} / {}", key.1, applied.len(), known.len()),
                )
                .clicked()
            {
                self.selected_rule = Some(key.clone());
            }
            if self.selected_rule.as_ref() != Some(&key) {
                continue;
            }
            for h in &targets {
                let state = if !known.contains(&h.name) {
                    "Нет данных"
                } else if !applied.contains(&h.name) {
                    "Нет правила"
                } else if self.exception {
                    "Есть исключение"
                } else {
                    "Забанен"
                };
                ui.label(format!("{}  ·  {state}", h.name));
            }
            self.action_button(
                ui,
                remove,
                remove_label,
                applied.clone(),
                &key.1,
                workers,
                kind,
            );
            let absent = known
                .iter()
                .filter(|n| !applied.contains(n))
                .cloned()
                .collect();
            if !known.iter().all(|n| applied.contains(n)) {
                self.action_button(ui, add, add_label, absent, &key.1, workers, kind);
            }
        }
    }
    fn entries(&self, name: &str) -> Option<&crate::catalog::Catalog> {
        self.catalogs.get(name)?.as_ref()?.as_ref().ok()
    }
    fn absent_hosts(&self, targets: &[&Host], url: &str) -> Vec<String> {
        targets
            .iter()
            .filter(|h| {
                self.entries(&h.name)
                    .is_some_and(|e| !e.keys().any(|(_, u)| u == url))
            })
            .map(|h| h.name.clone())
            .collect()
    }
    fn source_button(
        &mut self,
        ui: &mut Ui,
        id: &'static str,
        label: &'static str,
        names: Vec<String>,
        url: &str,
        workers: &HashMap<String, Worker>,
    ) {
        self.action_button(ui, id, label, names, url, workers, URL);
    }
    fn action_button(
        &mut self,
        ui: &mut Ui,
        id: &'static str,
        label: &'static str,
        names: Vec<String>,
        url: &str,
        workers: &HashMap<String, Worker>,
        kind: &'static str,
    ) {
        let names: Vec<_> = names
            .into_iter()
            .filter(|n| workers.contains_key(n) && !self.pending(n))
            .collect();
        let enabled = !names.is_empty() && cmd(id, label, kind).valid(url);
        let button = egui::Button::new(format!("{label} · {}", names.len()));
        if ui
            .add_enabled(enabled, button)
            .on_hover_text(names.join(", "))
            .clicked()
        {
            for name in names {
                let result = workers[&name].tx.send(Action::Manage {
                    action: id,
                    value: url.into(),
                });
                self.reports.insert(
                    (name, self.group),
                    Report {
                        label,
                        result: result.err().map(|_| Err("Сбор остановлен".into())),
                    },
                );
            }
        }
    }
}
