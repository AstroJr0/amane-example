use amane::{Color, Column, Parent, Rectangle, ScrollArea, Service, Text, Widget, children};

use crate::panel;
use crate::tray::{MenuEntry, MenuKind, Tray};

pub fn view(id: u32) -> Column {
    let tray = Tray::read();

    // the program quit while its menu was open
    let Some(item) = tray.items().iter().find(|item| item.id() == id) else {
        return message("this icon is gone");
    };

    if item.menu().is_empty() {
        return message("no menu");
    }

    let title = if item.title().is_empty() {
        item.app_id()
    } else {
        item.title()
    };

    let header = panel::header(title, "open", move |_| {
        Tray::activate(id, 0, 0);
        panel::close();
    });

    let mut rows: Vec<Box<dyn Widget>> = Vec::new();

    add_entries(id, item.menu(), 0, &mut rows);

    let list = ScrollArea::new("tray", Column::new(rows).width(Parent).gap(4.0));

    Column::new(children![header, list])
        .width(Parent)
        .height(Parent)
        .gap(12.0)
}

fn message(text: &str) -> Column {
    let text = Text::new(text).size(14.0).color(Color::from("#a6adc8"));

    Column::new(children![text])
}

// a submenu is shown right under its entry, one step in
fn add_entries(id: u32, entries: &[MenuEntry], depth: usize, rows: &mut Vec<Box<dyn Widget>>) {
    for entry in entries {
        rows.push(Box::new(row(id, entry, depth)));

        add_entries(id, entry.children(), depth + 1, rows);
    }
}

fn row(id: u32, entry: &MenuEntry, depth: usize) -> Rectangle {
    if entry.kind() == MenuKind::Separator {
        return Rectangle::new().width(Parent).height(1.0).fill("#45475a");
    }

    let label = format!("{}{}", "  ".repeat(depth), entry.label());

    let state = if !entry.enabled() {
        "disabled"
    } else if !entry.children().is_empty() {
        ">"
    } else {
        match entry.kind() {
            MenuKind::Check(true) | MenuKind::Radio(true) => "on",
            _ => "",
        }
    };

    let row = panel::item(&label, state);

    // a submenu opens by itself, and a disabled entry does nothing
    if !entry.enabled() || !entry.children().is_empty() {
        return row;
    }

    let entry = entry.id();

    row.on_click(move |_| {
        Tray::click_menu(id, entry);
        panel::close();
    })
}
