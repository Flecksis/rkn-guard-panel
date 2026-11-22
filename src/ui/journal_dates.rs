use super::*;
use chrono::{Datelike, Local, Months, NaiveDate, TimeZone};

type Bounds = (Option<i64>, Option<i64>);

#[derive(Default)]
pub(super) struct DateFilter {
    from: String,
    to: String,
    month: Option<NaiveDate>,
    error: Option<String>,
    pub bounds: Bounds,
}

impl DateFilter {
    pub fn show(&mut self, ui: &mut Ui) -> bool {
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            let mut picked = false;
            for (label, value) in [("С даты", &mut self.from), ("По дату", &mut self.to)]
            {
                ui.vertical(|ui| {
                    ui.label(label);
                    picked |= calendar(ui, label, value, &mut self.month);
                });
            }
            if picked {
                match bounds(&self.from, &self.to) {
                    Ok(bounds) => {
                        self.bounds = bounds;
                        self.error = None;
                        changed = true;
                    }
                    Err(error) => self.error = Some(error.to_string()),
                }
            }
            if ui.button("Сбросить даты").clicked() {
                *self = Self::default();
                changed = true;
            }
        });
        if let Some(error) = &self.error {
            ui.colored_label(C::LIGHT_RED, error);
        }
        if self.bounds != (None, None) {
            let display = |time: i64, format: &str| {
                Local
                    .timestamp_opt(time, 0)
                    .single()
                    .map(|time| time.format(format).to_string())
                    .unwrap_or_default()
            };
            let from = self.bounds.0.map(|time| display(time, "%d.%m.%Y"));
            let to = self.bounds.1.map(|time| display(time - 1, "%d.%m.%Y"));
            let summary = match (from, to) {
                (Some(from), Some(to)) => format!("Даты: {from} – {to}"),
                (Some(from), None) => format!("Даты: с {from}"),
                (None, Some(to)) => format!("Даты: по {to}"),
                _ => String::new(),
            };
            hint(ui, &summary);
        }
        changed
    }
}

fn calendar(ui: &mut Ui, label: &str, value: &mut String, month: &mut Option<NaiveDate>) -> bool {
    let today = Local::now().date_naive();
    let selected = NaiveDate::parse_from_str(value, "%d.%m.%Y").ok();
    let caption = selected
        .map(|d| d.format("%d.%m.%Y").to_string())
        .unwrap_or_else(|| "Выбрать дату".into());
    let button = ui.add_sized([146., 34.], egui::Button::new(caption));
    let id = ui.make_persistent_id(("calendar", label));
    if button.clicked() {
        *month = selected.unwrap_or(today).with_day(1);
        ui.memory_mut(|mem| mem.toggle_popup(id));
    }
    let mut picked = None;
    egui::popup_below_widget(
        ui,
        id,
        &button,
        egui::PopupCloseBehavior::CloseOnClickOutside,
        |ui| {
            let first = month.get_or_insert_with(|| today.with_day(1).unwrap());
            const MONTHS: [&str; 12] = [
                "Январь",
                "Февраль",
                "Март",
                "Апрель",
                "Май",
                "Июнь",
                "Июль",
                "Август",
                "Сентябрь",
                "Октябрь",
                "Ноябрь",
                "Декабрь",
            ];
            ui.horizontal(|ui| {
                if ui.button("‹").on_hover_text("Предыдущий месяц").clicked() {
                    *first = first.checked_sub_months(Months::new(1)).unwrap_or(*first);
                }
                ui.add_sized(
                    [150., 24.],
                    egui::Label::new(format!(
                        "{} {}",
                        MONTHS[first.month0() as usize],
                        first.year()
                    )),
                );
                if ui.button("›").on_hover_text("Следующий месяц").clicked() {
                    *first = first.checked_add_months(Months::new(1)).unwrap_or(*first);
                }
            });
            egui::Grid::new(id.with("days"))
                .min_col_width(25.)
                .spacing([3., 3.])
                .show(ui, |ui| {
                    for day in ["Пн", "Вт", "Ср", "Чт", "Пт", "Сб", "Вс"] {
                        ui.weak(day);
                    }
                    ui.end_row();
                    let offset = first.weekday().num_days_from_monday();
                    for _ in 0..offset {
                        ui.label("");
                    }
                    for day in 1..=31 {
                        let Some(date) = first.with_day(day) else {
                            break;
                        };
                        if ui
                            .selectable_label(selected == Some(date), day.to_string())
                            .clicked()
                        {
                            picked = Some(date);
                        }
                        if (day + offset) % 7 == 0 {
                            ui.end_row();
                        }
                    }
                });
            if ui.button("Сегодня").clicked() {
                picked = Some(today);
            }
        },
    );
    if let Some(date) = picked {
        *value = date.format("%d.%m.%Y").to_string();
        ui.memory_mut(|mem| mem.close_popup());
        return true;
    }
    false
}

fn bounds(from: &str, to: &str) -> Result<Bounds> {
    let parse = |text: &str| -> Result<Option<NaiveDate>> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(None);
        }
        let date = NaiveDate::parse_from_str(text, "%d.%m.%Y")
            .or_else(|_| NaiveDate::parse_from_str(text, "%Y-%m-%d"))
            .map_err(|_| anyhow::anyhow!("Введите дату в формате дд.мм.гггг."))?;
        Ok(Some(date))
    };
    let from = parse(from)?;
    let to = parse(to)?;
    anyhow::ensure!(
        !matches!((from, to), (Some(a), Some(b)) if a > b),
        "Начальная дата позже конечной."
    );
    let midnight = |day: NaiveDate| {
        Local
            .from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap())
            .earliest()
            .map(|stamp| stamp.timestamp())
            .ok_or_else(|| anyhow::anyhow!("Дата недоступна в местном часовом поясе."))
    };
    let end = to
        .map(|day| {
            let next = day
                .succ_opt()
                .ok_or_else(|| anyhow::anyhow!("Конечная дата вне диапазона."))?;
            midnight(next)
        })
        .transpose()?;
    Ok((from.map(midnight).transpose()?, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn фильтр_включает_весь_последний_день() -> Result<()> {
        let (start, end) = bounds("20.11.2026", "2026-11-20")?;
        let (start, end) = (start.unwrap(), end.unwrap());
        assert_eq!(
            Local
                .timestamp_opt(start, 0)
                .unwrap()
                .format("%d.%m %H:%M:%S")
                .to_string(),
            "20.11 00:00:00"
        );
        assert_eq!(
            Local
                .timestamp_opt(end - 1, 0)
                .unwrap()
                .format("%d.%m %H:%M:%S")
                .to_string(),
            "20.11 23:59:59"
        );
        assert!(bounds("21.11.2026", "20.11.2026").is_err());
        assert!(bounds("29.02.2026", "").is_err());
        assert_eq!(bounds("", "20.11.2026")?, (None, Some(end)));
        Ok(())
    }
}
