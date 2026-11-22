use super::*;

const BINS: usize = 60;
type Series = [Option<f64>; BINS];

struct History {
    values: [Series; 6],
    start: i64,
    end: i64,
}

impl History {
    fn load(db: &Connection, name: &str, hours: i64) -> rusqlite::Result<Self> {
        let end = chrono::Utc::now().timestamp() + 1;
        let start = end - hours * 3600;
        let step = (end - start) / BINS as i64;
        let mut values = [[None; BINS]; 6];
        let mut metrics = db.prepare(
            "SELECT (time-?2)/?4,avg(cpu),avg(ram),avg(rx),avg(tx)
             FROM metric_history WHERE host=?1 AND time>=?2 AND time<?3 GROUP BY 1",
        )?;
        let mut rows = metrics.query(params![name, start, end, step])?;
        while let Some(row) = rows.next()? {
            let index = row.get::<_, usize>(0)?;
            for (column, series) in values.iter_mut().take(4).enumerate() {
                series[index] = row.get(column + 1)?;
            }
        }
        let mut packets = db.prepare(
            "SELECT (time-?2)/?4,sum(dropped_packets) FROM samples
             WHERE host=?1 AND time>=?2 AND time<?3 GROUP BY 1",
        )?;
        let mut rows = packets.query(params![name, start, end, step])?;
        while let Some(row) = rows.next()? {
            values[4][row.get::<_, usize>(0)?] = row.get(1)?;
        }
        let mut archive = db.prepare(
            "SELECT (time-?2)/?4,count(*) FROM events
             WHERE host=?1 AND time>=?2 AND time<?3
             AND time<coalesce((SELECT min(time) FROM samples WHERE host=?1),?3)
             GROUP BY 1",
        )?;
        let mut rows = archive.query(params![name, start, end, step])?;
        while let Some(row) = rows.next()? {
            values[5][row.get::<_, usize>(0)?] = row.get(1)?;
        }
        Ok(Self { values, start, end })
    }

    fn chart(&self, ui: &mut Ui, title: &str, lines: &[(&str, usize, C)], unit: &str) {
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.heading(title);
            let peak = lines
                .iter()
                .flat_map(|(_, i, _)| self.values[*i].iter().flatten().copied())
                .fold(0., f64::max);
            let limit = if unit == "%" { 1. } else { peak.max(1.) };
            let format_value = |n| match unit {
                "%" => format!("{:.0}%", n * 100.),
                "сеть" => format!("{}/с", data_size(n)),
                _ => format!("{n:.0}"),
            };
            ui.horizontal_wrapped(|ui| {
                for (label, index, color) in lines {
                    let value = if unit == "пакеты" {
                        self.values[*index]
                            .iter()
                            .any(Option::is_some)
                            .then(|| self.values[*index].iter().flatten().sum())
                    } else {
                        self.values[*index].iter().rev().find_map(|v| *v)
                    };
                    ui.label(
                        T::new(format!(
                            "{label} {}",
                            value.map_or("—".into(), format_value)
                        ))
                        .color(*color),
                    );
                }
            });
            let (rect, response) = ui
                .allocate_exact_size(egui::vec2(ui.available_width(), 120.), egui::Sense::hover());
            let plot = rect.shrink2(egui::vec2(3., 6.));
            let painter = ui.painter();
            for fraction in [0., 0.5, 1.] {
                let y = egui::lerp(plot.y_range(), fraction);
                painter.line_segment(
                    [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
                    (1., LINE),
                );
            }
            if lines
                .iter()
                .all(|(_, i, _)| self.values[*i].iter().all(Option::is_none))
            {
                painter.text(
                    plot.center(),
                    egui::Align2::CENTER_CENTER,
                    "Пока нет замеров",
                    egui::FontId::proportional(14.),
                    MUTED,
                );
            }
            let width = plot.width() / BINS as f32;
            for (_, index, color) in lines {
                let mut previous = None;
                for (i, value) in self.values[*index].iter().enumerate() {
                    let x = plot.left() + width * (i as f32 + 0.5);
                    let current = value.map(|n| {
                        egui::pos2(
                            x,
                            plot.bottom() - plot.height() * (n / limit).clamp(0., 1.) as f32,
                        )
                    });
                    if let Some(point) = current {
                        if unit == "пакеты" {
                            let half = if *index == 4 { 1. } else { -1. };
                            painter.rect_filled(
                                egui::Rect::from_min_max(
                                    egui::pos2(
                                        x + width * (half * 0.21 - 0.18),
                                        point.y.min(plot.bottom() - 1.),
                                    ),
                                    egui::pos2(x + width * (half * 0.21 + 0.18), plot.bottom()),
                                ),
                                1.,
                                *color,
                            );
                        } else if let Some(last) = previous {
                            painter.line_segment([last, point], (2., *color));
                        } else {
                            painter.circle_filled(point, 2., *color);
                        }
                    }
                    previous = current;
                }
            }
            if let Some(pos) = response.hover_pos() {
                let index = (((pos.x - plot.left()) / width) as usize).min(BINS - 1);
                let x = plot.left() + width * (index as f32 + 0.5);
                painter.line_segment(
                    [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
                    (1., MUTED),
                );
                let stamp = self.start + (self.end - self.start) / BINS as i64 * index as i64;
                response.on_hover_ui(|ui| {
                    ui.label(when(stamp));
                    for (label, row, _) in lines {
                        ui.label(format!(
                            "{label} {}",
                            self.values[*row][index].map_or("—".into(), format_value)
                        ));
                    }
                });
            }
            ui.horizontal(|ui| {
                hint(ui, &when(self.start));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    hint(ui, &when(self.end - 1))
                });
            });
        });
    }
}

impl App {
    pub(super) fn server_detail(&mut self, ui: &mut Ui, name: &str) {
        ui.horizontal_wrapped(|ui| {
            if ui.button("← Назад").clicked() {
                self.detail = None;
            }
            title(ui, name);
            let busy = self.status.get(name).is_some_and(|status| {
                status.starts_with("Обновление · ") || status.starts_with("Удаление · ")
            });
            if ui
                .add_enabled(!busy, egui::Button::new("Изменить профиль"))
                .clicked()
            {
                if let Some(host) = self.hosts.iter().find(|host| host.name == name).cloned() {
                    self.open_profile(&host);
                }
            }
            ui.selectable_value(&mut self.day, false, "За час");
            ui.selectable_value(&mut self.day, true, "За сутки");
        });
        ui.add_space(10.);
        let resource_hours = if self.day { 24 } else { 1 };
        let history = match History::load(&self.db, name, resource_hours) {
            Ok(history) => history,
            Err(err) => {
                ui.colored_label(C::LIGHT_RED, err.to_string());
                return;
            }
        };
        egui::ScrollArea::vertical()
            .id_salt(("server-detail", name))
            .show(ui, |ui| {
                if ui.available_width() >= 620. {
                    ui.columns(2, |cols| {
                        history.chart(
                            &mut cols[0],
                            "Загрузка CPU",
                            &[("CPU", 0, glass::CYAN)],
                            "%",
                        );
                        history.chart(
                            &mut cols[1],
                            "Оперативная память",
                            &[("RAM", 1, glass::BLUE)],
                            "%",
                        );
                    });
                } else {
                    history.chart(ui, "Загрузка CPU", &[("CPU", 0, glass::CYAN)], "%");
                    history.chart(ui, "Оперативная память", &[("RAM", 1, glass::BLUE)], "%");
                }
                history.chart(
                    ui,
                    "Сеть",
                    &[("Приём", 2, glass::CYAN), ("Передача", 3, glass::BLUE)],
                    "сеть",
                );
                let mut packet_hours = self.packet_hours.unwrap_or(resource_hours);
                ui.horizontal_wrapped(|ui| {
                    ui.label("Период для пакетов");
                    for (hours, label) in [(1, "За час"), (24, "За сутки"), (168, "За 7 дней")]
                    {
                        if ui
                            .selectable_value(&mut packet_hours, hours, label)
                            .clicked()
                        {
                            self.packet_hours = Some(hours);
                        }
                    }
                });
                let packet_history = (packet_hours != resource_hours)
                    .then(|| History::load(&self.db, name, packet_hours));
                match packet_history
                    .as_ref()
                    .map_or(Ok(&history), |result| result.as_ref())
                {
                    Ok(packets) => packets.chart(
                        ui,
                        "Отброшенные пакеты",
                        &[
                            ("По счётчикам", 4, glass::ACCENT),
                            ("До первого замера, по журналу", 5, glass::CYAN),
                        ],
                        "пакеты",
                    ),
                    Err(err) => {
                        ui.colored_label(C::LIGHT_RED, err.to_string());
                    }
                }
                if let Err(err) = self.attack_table(ui, Some(name)) {
                    ui.colored_label(C::LIGHT_RED, err.to_string());
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn история_разделяет_серверы_и_периоды() -> Result<()> {
        let db = Connection::open_in_memory()?;
        db.execute_batch(include_str!("../schema.sql"))?;
        let now = chrono::Utc::now().timestamp();
        db.execute(
            "INSERT INTO metric_history VALUES('a',?,0.5,0.3,100,200)",
            [now - 20],
        )?;
        db.execute(
            "INSERT INTO metric_history VALUES('b',?,1.,1.,900,900)",
            [now - 20],
        )?;
        db.execute(
            "INSERT INTO samples VALUES('a',?,'4','boot',10,100,7,80)",
            [now - 7200],
        )?;
        for time in [now - 2 * 86400, now - 7300, now - 10] {
            db.execute(
                "INSERT INTO events VALUES('a',?,'192.0.2.1','TCP',443,'test')",
                [time],
            )?;
        }
        let hour = History::load(&db, "a", 1)?;
        assert_eq!(
            hour.values[0].iter().flatten().copied().collect::<Vec<_>>(),
            vec![0.5]
        );
        assert!(hour.values[4].iter().all(Option::is_none));
        assert!(hour.values[5].iter().all(Option::is_none));
        let day = History::load(&db, "a", 24)?;
        assert_eq!(day.values[4].iter().flatten().sum::<f64>(), 7.);
        assert_eq!(day.values[5].iter().flatten().sum::<f64>(), 1.);
        let week = History::load(&db, "a", 168)?;
        assert_eq!(week.values[5].iter().flatten().sum::<f64>(), 2.);
        Ok(())
    }
}
