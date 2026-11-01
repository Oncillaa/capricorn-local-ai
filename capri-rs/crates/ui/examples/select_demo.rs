//! Проверка шторки выделения области.
//!
//! Запуск: `cargo run -p capri-ui --example select_demo`
//!
//! Появится затемнённый экран. Тяни мышью прямоугольник, `Esc` или правая
//! кнопка — отмена. В консоль печатается то, что вернула функция.

fn main() {
    println!("сейчас появится шторка. Тяни мышью область, Esc — отмена.");
    match capri_ui::select::select_region() {
        Some(capri_ui::select::Pick::Region { x, y, width, height }) => {
            println!("выделено: x={x}, y={y}, {width}x{height}");
            if width < 4 || height < 4 {
                println!("ВНИМАНИЕ: область слишком маленькая — так быть не должно");
            }
        }
        Some(capri_ui::select::Pick::Cancelled) => println!("отменено"),
        None => println!("шторка не создалась"),
    }
}
