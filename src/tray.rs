mod icon;
mod item;
mod menu;
mod watcher;

use std::thread;

use amane::{Argument, Bus, Service, arguments};

pub use item::{TrayItem, TrayStatus};
pub use menu::{MenuEntry, MenuKind};

pub struct Tray {
    // in the order the programs registered
    items: Vec<TrayItem>,

    running: bool,
}

// the shell is the tray -> programs register their icons with it.
// it is the watcher and the host in one, so there is
// nothing to instal, only amane's D-Bus wrapper
impl Service for Tray {
    fn new() -> Self {
        Self {
            items: Vec::new(),
            running: false,
        }
    }

    fn listen() {
        icon::warm_up();

        watcher::run();
    }
}

impl Tray {
    // every icon, whatever its status, so hide the passive ones yourself
    pub fn items(&self) -> &[TrayItem] {
        &self.items
    }

    // false when another tray, like waybar's or snixembed, already owns the watcher name
    pub fn running(&self) -> bool {
        self.running
    }

    // x and y are where on the screen the click happened; programs use them
    // to place a window they open, and 0, 0 is fine when that does not matter
    pub fn activate(id: u32, x: i32, y: i32) {
        call(id, "Activate", arguments![x, y]);
    }

    // the middle click
    pub fn secondary_activate(id: u32, x: i32, y: i32) {
        call(id, "SecondaryActivate", arguments![x, y]);
    }

    // asks the program to show its own menu, which on wayland not every program can do
    pub fn context_menu(id: u32, x: i32, y: i32) {
        call(id, "ContextMenu", arguments![x, y]);
    }

    // a scroll over the icon, where the delta is the steps scrolled
    pub fn scroll(id: u32, delta: i32, horizontal: bool) {
        let orientation = if horizontal { "horizontal" } else { "vertical" };

        call(id, "Scroll", arguments![delta, orientation]);
    }

    // a menu entry was clicked, pass the id from entry.id()
    pub fn click_menu(id: u32, entry: i32) {
        let Some((owner, path)) = Self::menu_of(id) else {
            return;
        };

        thread::spawn(move || menu::click(&owner, &path, entry));
    }

    // the item's bus name and the path of its menu, when it has one
    fn menu_of(id: u32) -> Option<(String, String)> {
        let tray = Self::read();

        let item = tray.items.iter().find(|item| item.id == id)?;

        if item.menu_path.is_empty() {
            return None;
        }

        Some((item.owner.clone(), item.menu_path.clone()))
    }

    fn replace(&mut self, item: TrayItem) {
        let Some(old) = self.items.iter_mut().find(|old| old.id == item.id) else {
            // it quit while its new properties were being read
            remove_icons(&item);

            return;
        };

        // icons that changed leave their old files behind, so those go
        for (before, after) in old.icons().into_iter().zip(item.icons()) {
            if let Some(before) = before
                && Some(before) != after
            {
                icon::remove(before);
            }
        }

        *old = item;
    }

    // the registered names of the items that were removed, so the watcher can announce them
    fn remove_owner(&mut self, owner: &str) -> Vec<String> {
        let mut removed = Vec::new();

        self.items.retain(|item| {
            if item.owner != owner {
                return true;
            }

            remove_icons(item);

            removed.push(item.registered.clone());

            false
        });

        removed
    }
}

fn remove_icons(item: &TrayItem) {
    for icon in item.icons().into_iter().flatten() {
        icon::remove(icon);
    }
}

// sent from a thread of its own, so a program that is slow to answer never stalls drawing
fn call(id: u32, method: &'static str, arguments: Vec<Argument>) {
    let target = {
        let tray = Tray::read();

        tray.items
            .iter()
            .find(|item| item.id == id)
            .map(|item| (item.owner.clone(), item.path.clone()))
    };

    let Some((owner, path)) = target else {
        return;
    };

    thread::spawn(move || {
        Bus::session().call(&owner, &path, item::INTERFACE, method, &arguments);
    });
}
