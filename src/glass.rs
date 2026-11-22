use egui::{Color32 as C, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};

pub const ACCENT: C = C::from_rgb(169, 200, 214);
pub const CYAN: C = C::from_rgb(132, 184, 170);
pub const BLUE: C = C::from_rgb(155, 172, 186);

pub fn profile_row<R>(ui: &mut Ui, content: impl FnOnce(&mut Ui) -> R) -> egui::InnerResponse<R> {
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 30.),
        egui::Layout::right_to_left(egui::Align::Min),
        content,
    )
}

pub fn profile_icon(ui: &mut Ui, edit: bool, revealed: bool, label: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(30., 30.), Sense::click());
    let visuals = ui.style().interact(&response);
    let color = if revealed {
        ACCENT
    } else {
        visuals.fg_stroke.color
    };
    let painter = ui.painter();
    if response.hovered() || response.has_focus() {
        painter.rect_filled(rect, 8., visuals.weak_bg_fill);
    }
    let at = |x, y| rect.center() + vec2(x, y);
    let stroke = Stroke::new(1.5_f32, color);
    if edit {
        painter.add(egui::Shape::closed_line(
            vec![
                at(-7., 7.),
                at(-6., 2.),
                at(3., -7.),
                at(7., -3.),
                at(-2., 6.),
            ],
            stroke,
        ));
        painter.line_segment([at(1., -5.), at(5., -1.)], stroke);
    } else {
        painter.add(egui::Shape::closed_line(
            vec![
                at(-8., 0.),
                at(-4., -4.),
                at(0., -5.),
                at(4., -4.),
                at(8., 0.),
                at(4., 4.),
                at(0., 5.),
                at(-4., 4.),
            ],
            stroke,
        ));
        painter.circle_stroke(rect.center(), 2.3, stroke);
        if revealed {
            painter.line_segment([at(-7., 7.), at(7., -7.)], stroke);
        }
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    response.on_hover_text(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn строка_профиля_не_растягивается_под_высоту_окна() {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900., 800.))),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let row = profile_row(ui, |ui| {
                        profile_icon(ui, true, false, "Изменить профиль");
                        profile_icon(ui, false, false, "Показать адрес");
                        ui.vertical(|ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal_wrapped(|ui| ui.label("node-pl"));
                        });
                    });
                    assert!(row.response.rect.height() <= 40.);
                });
            });
        });
    }
}

pub fn glyph(p: &egui::Painter, center: Pos2, index: usize, color: C) {
    let at = |x: f32, y: f32| center + vec2(x, y);
    let stroke = Stroke::new(1.55_f32, color);
    let line = |pts: &[(f32, f32)]| {
        p.add(egui::Shape::line(
            pts.iter().map(|&(x, y)| at(x, y)).collect(),
            stroke,
        ));
    };
    match index {
        0 => line(&[
            (-9., 1.),
            (-5., 1.),
            (-2., -6.),
            (1., 7.),
            (4., -2.),
            (6., 1.),
            (9., 1.),
        ]),
        1 => {
            for y in [-5., 3.] {
                p.rect_stroke(
                    Rect::from_min_size(at(-8., y), vec2(16., 6.)),
                    2.,
                    stroke,
                    egui::StrokeKind::Inside,
                );
                p.circle_filled(at(4., y + 3.), 1., color);
            }
        }
        2 => {
            p.rect_stroke(
                Rect::from_center_size(center, vec2(14., 17.)),
                2.,
                stroke,
                egui::StrokeKind::Inside,
            );
            for y in [-4., 0., 4.] {
                line(&[(-3., y), (3., y)]);
            }
        }
        _ => {
            for (x, h) in [(-6., 5.), (0., 10.), (6., 15.)] {
                p.rect_filled(
                    Rect::from_min_size(at(x - 1.5, 7. - h), vec2(3., h)),
                    1.,
                    color,
                );
            }
        }
    }
}
pub fn nav(ui: &mut Ui, label: &str, index: usize, selected: bool) -> bool {
    let (r, response) = ui.allocate_exact_size(vec2(ui.available_width(), 42.), Sense::click());
    let p = ui.painter();
    let color = if selected {
        ACCENT
    } else {
        C::from_rgb(180, 186, 207)
    };
    if selected || response.hovered() {
        p.rect_filled(
            r,
            9.,
            if selected {
                C::from_rgb(48, 56, 61)
            } else {
                C::from_rgb(40, 44, 47)
            },
        );
    }
    if selected {
        p.rect_filled(
            Rect::from_center_size(pos2(r.left() + 1.5, r.center().y), vec2(3., 16.)),
            2.,
            ACCENT,
        );
    }
    glyph(p, pos2(r.left() + 22., r.center().y), index, color);
    p.text(
        pos2(r.left() + 42., r.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(15.),
        color,
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, label)
    });
    response.clicked()
}
