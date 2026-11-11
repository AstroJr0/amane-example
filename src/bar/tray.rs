use amane::{Button, Center, Image, Pointer, Rectangle, Row, Scroll, Service, Widget};

use crate::panel::{self, Panel};
use crate::tray::{Tray, TrayItem, TrayStatus};

pub fn view() -> Row {
    let tray = Tray::read();

    let mut icons: Vec<Box<dyn Widget>> = Vec::new();

    for item in tray.items() {
        // passive items have nothing to say, so the bar hides them
        if item.status() != TrayStatus::Passive {
            icons.push(Box::new(icon(item)));
        }
    }

    Row::new(icons).gap(8.0).align(Center)
}

fn icon(item: &TrayItem) -> Rectangle {
    let id = item.id();
    let is_menu = item.is_menu();
    let has_menu = !item.menu().is_empty();

    let picture = Rectangle::new().width(20.0).height(20.0);

    // a program with no icon at all still gets a place to click
    let picture = match item.icon_path() {
        Some(path) => picture.fill(Image::contain(path)),
        None => picture.fill("#45475a").radius(10.0),
    };

    Rectangle::new()
        .width(24.0)
        .height(24.0)
        .radius(6.0)
        .cursor(Pointer)
        .align_child(Center, Center)
        .on_click(move |button| clicked(id, button, is_menu, has_menu))
        .on_scroll(move |scroll| scrolled(id, scroll))
        .child(picture)
}

fn clicked(id: u32, button: Button, is_menu: bool, has_menu: bool) {
    match button {
        // an item that is only a menu has nothing to activate
        Button::Left if !is_menu => Tray::activate(id, 0, 0),
        Button::Middle => Tray::secondary_activate(id, 0, 0),
        // a right click, or a left click on a menu
        _ if has_menu => panel::toggle(Panel::Tray(id)),
        // the program's own menu, when it has none we can draw
        _ => Tray::context_menu(id, 0, 0),
    }
}

// positive is down, like the scroll it comes from
fn scrolled(id: u32, scroll: Scroll) {
    if scroll.y != 0.0 {
        Tray::scroll(id, scroll.y.signum() as i32, false);
    }

    if scroll.x != 0.0 {
        Tray::scroll(id, scroll.x.signum() as i32, true);
    }
}
