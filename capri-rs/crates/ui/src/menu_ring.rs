//! Кольцо радиального меню: геометрия и отрисовка.
//!
//! Всё, что можно посчитать без Windows, живёт здесь — поэтому и тесты
//! настоящие, без окон. Окно, хуки и показ — в [`crate::radial`].

use crate::canvas::{Canvas, Mask};

/// Сторона окна меню. Кольцо занимает 464 px, остальное — запас под свечение.
pub const SIZE: i32 = 520;
/// Внешний радиус кольца.
pub const R_OUTER: f32 = 232.0;
/// Внутренний радиус: ближе к центру — уже не сектор.
pub const R_INNER: f32 = 148.0;
/// Радиус центрального кружка с крестиком.
pub const R_CENTER: f32 = 56.0;
/// Сколько секторов.
pub const SECTORS: usize = 8;

pub const VIOLET: [u8; 3] = [176, 38, 255];
pub const GREEN: [u8; 3] = [40, 255, 150];
/// Подложка невыбранного сектора.
pub const IDLE: [u8; 3] = [34, 20, 58];
/// Цвет подписей.
pub const TEXT: [u8; 3] = [214, 200, 255];
/// Фон окна (он же центр).
pub const BG: [u8; 3] = [10, 6, 18];

/// Куда попал курсор.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// Мимо кольца — клик закрывает меню.
    Outside,
    /// Между центром и кольцом — пусто, ничего не происходит.
    Gap,
    /// Центральный крестик — закрыть.
    Center,
    /// Сектор с номером 0…7.
    Sector(usize),
}

/// Попадание по смещению от центра окна.
pub fn hit(dx: f32, dy: f32) -> Hit {
    let distance = (dx * dx + dy * dy).sqrt();
    if distance > R_OUTER {
        return Hit::Outside;
    }
    if distance <= R_CENTER {
        return Hit::Center;
    }
    if distance < R_INNER {
        return Hit::Gap;
    }
    Hit::Sector(sector_index(dx, dy))
}

/// Номер сектора по смещению: 0 — верхний, дальше по часовой стрелке.
pub fn sector_index(dx: f32, dy: f32) -> usize {
    // `atan2` считает от направления «3 часа» и растёт по часовой стрелке
    // (ось Y смотрит вниз). Прибавляем 90°, чтобы ноль стал верхом.
    let mut degrees = dy.atan2(dx).to_degrees() + 90.0;
    if degrees < 0.0 {
        degrees += 360.0;
    }
    ((degrees / (360.0 / SECTORS as f32)) as usize) % SECTORS
}

/// Угол середины сектора в радианах — туда ставим подпись.
pub fn sector_angle(index: usize) -> f32 {
    let step = 360.0 / SECTORS as f32;
    (index as f32 * step + step / 2.0 - 90.0).to_radians()
}

/// Нарисовать кольцо. `hover` — сектор под курсором, `labels` — подписи.
pub fn paint(canvas: &mut Canvas, hover: Option<Hit>, labels: &[&str]) {
    canvas.clear();
    let center = SIZE as f32 / 2.0;

    // Свечение вокруг кольца: два мягких обода.
    ring(canvas, center, center, R_OUTER + 2.0, R_OUTER + 10.0, VIOLET, 0.12);

    // Сектора.
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 + 0.5 - center;
            let dy = y as f32 + 0.5 - center;
            let distance = (dx * dx + dy * dy).sqrt();
            if distance < R_INNER || distance > R_OUTER {
                continue;
            }
            let hot = hover == Some(Hit::Sector(sector_index(dx, dy)));
            let (color, alpha) = if hot { (VIOLET, 0.42) } else { (IDLE, 0.84) };
            canvas.blend(x as u32, y as u32, color[0], color[1], color[2], alpha);
        }
    }

    // Кромки кольца.
    ring(canvas, center, center, R_OUTER - 2.0, R_OUTER, VIOLET, 0.95);
    ring(canvas, center, center, R_INNER, R_INNER + 1.5, VIOLET, 0.5);

    // Разделители секторов.
    for index in 0..SECTORS {
        let angle = (index as f32 * (360.0 / SECTORS as f32) - 90.0).to_radians();
        let (sin, cos) = angle.sin_cos();
        line(
            canvas,
            (center + cos * R_INNER, center + sin * R_INNER),
            (center + cos * R_OUTER, center + sin * R_OUTER),
            1.5,
            VIOLET,
            0.45,
        );
    }

    // Центр: тёмный кружок с крестиком «закрыть».
    let center_hot = hover == Some(Hit::Center);
    disc(canvas, center, center, R_CENTER, BG, 0.94);
    ring(
        canvas,
        center,
        center,
        R_CENTER - 1.5,
        R_CENTER,
        if center_hot { GREEN } else { VIOLET },
        0.9,
    );
    let cross = if center_hot { GREEN } else { TEXT };
    let arm = R_CENTER * 0.40;
    line(canvas, (center - arm, center - arm), (center + arm, center + arm), 2.5, cross, 0.95);
    line(canvas, (center - arm, center + arm), (center + arm, center - arm), 2.5, cross, 0.95);

    // Подписи и номера секторов.
    for (index, label) in labels.iter().enumerate().take(SECTORS) {
        let hot = hover == Some(Hit::Sector(index));
        let (sin, cos) = sector_angle(index).sin_cos();

        let radius = (R_INNER + R_OUTER) / 2.0 + 4.0;
        let mask = render_text(label, 12, hot, 104);
        place_centered(
            canvas,
            &mask,
            center + cos * radius,
            center + sin * radius,
            if hot { GREEN } else { TEXT },
            0.95,
        );

        // Номер — ближе к краю, чтобы было видно, какой цифрой звать.
        let number = (index + 1).to_string();
        let digit = render_text(&number, 10, false, 24);
        place_centered(
            canvas,
            &digit,
            center + cos * (R_OUTER - 13.0),
            center + sin * (R_OUTER - 13.0),
            VIOLET,
            0.8,
        );
    }
}

/// Подписи секторов из списка ядра.
pub fn labels_of(sectors: &[capri_core::menu::Sector]) -> Vec<&'static str> {
    sectors.iter().map(|sector| sector.label).collect()
}

/// Текст в маску: на Windows — настоящий шрифт, на других ОС — пусто.
#[cfg(windows)]
fn render_text(text: &str, size: i32, bold: bool, width: i32) -> Mask {
    crate::text::render(text, size, bold, width)
}

#[cfg(not(windows))]
fn render_text(_text: &str, _size: i32, _bold: bool, _width: i32) -> Mask {
    Mask::empty()
}

fn place_centered(canvas: &mut Canvas, mask: &Mask, cx: f32, cy: f32, color: [u8; 3], alpha: f32) {
    if mask.width == 0 || mask.height == 0 {
        return;
    }
    let left = cx - mask.width as f32 / 2.0;
    let top = cy - mask.height as f32 / 2.0;
    canvas.draw_mask(mask, left.round() as i32, top.round() as i32, color, alpha);
}

/// Круг из центра.
fn disc(canvas: &mut Canvas, cx: f32, cy: f32, radius: f32, color: [u8; 3], alpha: f32) {
    circle_part(canvas, cx, cy, 0.0, radius, color, alpha);
}

/// Кольцо между двумя радиусами.
fn ring(
    canvas: &mut Canvas,
    cx: f32,
    cy: f32,
    inner: f32,
    outer: f32,
    color: [u8; 3],
    alpha: f32,
) {
    circle_part(canvas, cx, cy, inner.max(0.0), outer, color, alpha);
}

fn circle_part(
    canvas: &mut Canvas,
    cx: f32,
    cy: f32,
    inner: f32,
    outer: f32,
    color: [u8; 3],
    alpha: f32,
) {
    let left = (cx - outer).floor().max(0.0) as i32;
    let right = ((cx + outer).ceil() as i32).min(SIZE);
    let top = (cy - outer).floor().max(0.0) as i32;
    let bottom = ((cy + outer).ceil() as i32).min(SIZE);
    for y in top..bottom {
        for x in left..right {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let distance = (dx * dx + dy * dy).sqrt();
            if distance < inner || distance > outer {
                continue;
            }
            canvas.blend(x as u32, y as u32, color[0], color[1], color[2], alpha);
        }
    }
}

/// Отрезок толщиной `width` (сглаживания нет — на неоне не нужно).
fn line(
    canvas: &mut Canvas,
    (x0, y0): (f32, f32),
    (x1, y1): (f32, f32),
    width: f32,
    color: [u8; 3],
    alpha: f32,
) {
    let steps = ((x1 - x0).abs().max((y1 - y0).abs())).max(1.0) as i32;
    let half = (width / 2.0).max(0.5);
    for step in 0..=steps {
        let t = step as f32 / steps as f32;
        let x = x0 + (x1 - x0) * t;
        let y = y0 + (y1 - y0) * t;
        for oy in -(half.ceil() as i32)..=(half.ceil() as i32) {
            for ox in -(half.ceil() as i32)..=(half.ceil() as i32) {
                let px = x.round() as i32 + ox;
                let py = y.round() as i32 + oy;
                if px < 0 || py < 0 || px >= SIZE || py >= SIZE {
                    continue;
                }
                canvas.blend(px as u32, py as u32, color[0], color[1], color[2], alpha);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha_at(canvas: &Canvas, x: i32, y: i32) -> u32 {
        (canvas.pixels[(y as u32 * canvas.width + x as u32) as usize] >> 24) & 0xFF
    }

    #[test]
    fn top_is_the_first_sector_and_the_ring_goes_clockwise() {
        // Верх — первый пункт («Настройки»).
        assert_eq!(hit(0.0, -200.0), Hit::Sector(0));
        // Дальше по часовой стрелке: вправо — третий по счёту (номер 2).
        assert_eq!(hit(200.0, 0.0), Hit::Sector(2));
        // Низ — пятый, влево — седьмой.
        assert_eq!(hit(0.0, 200.0), Hit::Sector(4));
        assert_eq!(hit(-200.0, 0.0), Hit::Sector(6));
    }

    #[test]
    fn every_sector_is_reachable_from_its_own_middle() {
        for index in 0..SECTORS {
            let (sin, cos) = sector_angle(index).sin_cos();
            let radius = (R_INNER + R_OUTER) / 2.0;
            assert_eq!(hit(cos * radius, sin * radius), Hit::Sector(index), "сектор {index}");
        }
    }

    #[test]
    fn center_gap_and_outside_are_told_apart() {
        assert_eq!(hit(0.0, 0.0), Hit::Center);
        assert_eq!(hit(10.0, 10.0), Hit::Center);
        // Между крестиком и кольцом — пусто.
        assert_eq!(hit(0.0, 100.0), Hit::Gap);
        // За кольцом — «мимо».
        assert_eq!(hit(0.0, 400.0), Hit::Outside);
        assert_eq!(hit(300.0, 300.0), Hit::Outside);
    }

    #[test]
    fn ring_is_drawn_where_expected_and_nothing_beyond_it() {
        let mut canvas = Canvas::new(SIZE as u32, SIZE as u32);
        let labels = ["а", "б", "в", "г", "д", "е", "ж", "з"];
        paint(&mut canvas, None, &labels);
        let center = SIZE / 2;

        // Внутри кольца — видимо.
        assert!(alpha_at(&canvas, center, center - 200) > 40, "сектор сверху пустой");
        // За внешним радиусом и свечением — прозрачно, иначе окно было бы квадратом.
        assert_eq!(alpha_at(&canvas, 4, 4), 0);
        assert_eq!(alpha_at(&canvas, SIZE - 5, SIZE - 5), 0);
        // Центр закрашен тёмным (смотрим мимо крестика — он как раз в середине).
        assert!(alpha_at(&canvas, center, center + 30) > 200, "центр должен быть плотным");
        let blue = canvas.pixels[((center + 30) * SIZE + center) as usize] & 0xFF;
        assert!(blue < 60, "подложка центра должна быть тёмной, а не синей: {blue}");
    }

    #[test]
    fn hovered_sector_becomes_brighter() {
        let labels = ["а", "б", "в", "г", "д", "е", "ж", "з"];
        let mut idle = Canvas::new(SIZE as u32, SIZE as u32);
        paint(&mut idle, None, &labels);
        let mut hot = Canvas::new(SIZE as u32, SIZE as u32);
        paint(&mut hot, Some(Hit::Sector(0)), &labels);

        // Берём точку в верхнем секторе, подальше от подписи и разделителей.
        let center = SIZE / 2;
        let index = ((center - 205) * SIZE + center) as usize;
        let total = |pixel: u32| (pixel & 0xFF) + ((pixel >> 8) & 0xFF) + ((pixel >> 16) & 0xFF);
        assert!(
            total(hot.pixels[index]) > total(idle.pixels[index]),
            "подсвеченный сектор должен быть заметно ярче"
        );
    }

    #[test]
    fn labels_come_from_the_core_list() {
        let labels = labels_of(&capri_core::menu::SECTORS);
        assert_eq!(labels.len(), SECTORS);
        assert_eq!(labels[0], "Настройки");
    }
}
