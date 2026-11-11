use std::collections::BTreeMap;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Once, mpsc};
use std::thread;
use std::time::Duration;

use amane::{Argument, Bus, Method, Service, Signal};

use super::item::{self, TrayItem};
use super::{Tray, menu};

const NAME: &str = "org.kde.StatusNotifierWatcher";

const PATH: &str = "/StatusNotifierWatcher";

const INTERFACE: &str = "org.kde.StatusNotifierWatcher";

const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

// where an item lives when it registers with only its bus name
const DEFAULT_PATH: &str = "/StatusNotifierItem";

// what a program announces when its own item changes
const ITEM_CHANGES: [&str; 6] = [
    "NewTitle",
    "NewIcon",
    "NewAttentionIcon",
    "NewOverlayIcon",
    "NewToolTip",
    "NewStatus",
];

// and when its menu does
const MENU_CHANGES: [&str; 2] = ["LayoutUpdated", "ItemsPropertiesUpdated"];

// how long a helper that panicked waits before it listens again
const RESTART_DELAY: Duration = Duration::from_secs(5);

// the numbers stay apart across restarts of the helpers, so a file never changes owner
static NEXT_ID: AtomicU32 = AtomicU32::new(1);

static HELPERS: Once = Once::new();

/*
 * the shell is the watcher and the host at once: programs register their items
 * with it, and it keeps them to show. it never takes the name from another
 * watcher, so a bar that already runs one has to be stopped
 */
pub fn run() {
    let bus = Bus::session();

    // watched before owning the name, so no early call is missed
    let methods = bus.methods(PATH, INTERFACE);

    watch_properties();

    if !bus.own(NAME) {
        return;
    }

    Tray::write().running = true;

    // a restart must not start a second set of helpers
    HELPERS.call_once(watch_changes);

    // programs that started first only look for a host when they hear this
    bus.emit(PATH, INTERFACE, "StatusNotifierHostRegistered", &[]);

    for method in methods {
        answer(&method);
    }
}

// programs ask for the items and the host with property reads, on a thread of their own
fn watch_properties() {
    static WATCHING: Once = Once::new();

    WATCHING.call_once(|| {
        let (ready, watching) = mpsc::channel();

        keep(move || {
            let properties = Bus::session().methods(PATH, PROPERTIES);

            // only the first start is waited for, so a failed send changes nothing
            let _ = ready.send(());

            for method in properties {
                answer_property(&method);
            }
        });

        // no call can arrive before the name is owned, and it is not owned before this
        let _ = watching.recv();
    });
}

fn watch_changes() {
    for name in ITEM_CHANGES {
        keep(move || {
            for signal in Bus::session().signals(item::INTERFACE, name) {
                reload(&signal, false);
            }
        });
    }

    for name in MENU_CHANGES {
        keep(move || {
            for signal in Bus::session().signals(menu::INTERFACE, name) {
                reload(&signal, true);
            }
        });
    }

    keep(|| {
        for signal in Bus::session().signals("org.freedesktop.DBus", "NameOwnerChanged") {
            departed(&signal);
        }
    });
}

// a helper that panics starts again after a pause, and one that returns is done
fn keep(helper: impl Fn() + Send + 'static) {
    thread::spawn(move || {
        while panic::catch_unwind(AssertUnwindSafe(&helper)).is_err() {
            eprintln!("tray: stopped listening, starting it again");

            thread::sleep(RESTART_DELAY);
        }
    });
}

fn answer(method: &Method) {
    match method.name() {
        "RegisterStatusNotifierItem" => {
            // answered first, because the program waits for it before it serves its item
            method.reply(&[]);

            if let [service] = method.arguments() {
                register(service.text(), method.sender());
            }
        }

        // the shell is the host itself, so there is nothing to remember
        "RegisterStatusNotifierHost" => method.reply(&[]),

        // every call waits for an answer, even one the shell does not know
        _ => method.reply(&[]),
    }
}

fn register(service: &str, sender: &str) {
    let (name, path) = address(service, sender);

    // also proves the name exists, since a program that already left has no owner
    let owner = owner(&name);

    if owner.is_empty() {
        return;
    }

    let known = Tray::read()
        .items
        .iter()
        .any(|item| item.owner == owner && item.path == path);

    if known {
        return;
    }

    let registered = registered(&name, &path);

    let mut item = TrayItem::new(
        NEXT_ID.fetch_add(1, Ordering::Relaxed),
        registered.clone(),
        owner,
        path,
    );

    // asked outside the lock, because the program may be slow to answer
    item.refresh();

    Tray::write().items.push(item);

    announce("StatusNotifierItemRegistered", &registered);
}

/*
 * a program registers with its bus name, or like libappindicator with only
 * an object path, which means its own connection, so the caller is the name
 */
fn address(service: &str, sender: &str) -> (String, String) {
    if service.starts_with('/') {
        return (String::from(sender), String::from(service));
    }

    // some programs join the two, like "org.example.Item/MenuBar"
    match service.split_once('/') {
        Some((name, path)) => (String::from(name), format!("/{path}")),
        None => (String::from(service), String::from(DEFAULT_PATH)),
    }
}

// how other hosts expect an item to be named: the bus name, and the path unless it is the usual one
fn registered(name: &str, path: &str) -> String {
    if path == DEFAULT_PATH {
        String::from(name)
    } else {
        format!("{name}{path}")
    }
}

// the unique name that holds a bus name, or nothing when nobody does
fn owner(name: &str) -> String {
    let owner = Bus::session().call(
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "GetNameOwner",
        &[Argument::from(name)],
    );

    String::from(owner.text())
}

fn announce(signal: &str, registered: &str) {
    Bus::session().emit(PATH, INTERFACE, signal, &[Argument::from(registered)]);
}

// an item's own signal, from its program's unique name and on its own path
fn reload(signal: &Signal, menu: bool) {
    let found = Tray::read()
        .items
        .iter()
        .find(|item| {
            let path = if menu { &item.menu_path } else { &item.path };

            item.owner == signal.sender() && path == signal.path()
        })
        .cloned();

    let Some(mut item) = found else {
        return;
    };

    if menu {
        item.reload_menu();
    } else {
        item.refresh();
    }

    // an announcement that changed nothing a window shows redraws nothing
    if Tray::read().items.contains(&item) {
        return;
    }

    Tray::write().replace(item);
}

// a program that quit takes its items with it, and says nothing
fn departed(signal: &Signal) {
    let [_, gone, arrived] = signal.arguments() else {
        return;
    };

    if !arrived.text().is_empty() {
        return;
    }

    let gone = gone.text();

    // most names that leave the bus are not tray items
    if !Tray::read().items.iter().any(|item| item.owner == gone) {
        return;
    }

    let removed = Tray::write().remove_owner(gone);

    for registered in removed {
        announce("StatusNotifierItemUnregistered", &registered);
    }
}

fn answer_property(method: &Method) {
    match (method.name(), method.arguments()) {
        ("Get", [_, name]) => match property(name.text()) {
            Some(value) => method.reply(&[Argument::Variant(Box::new(value))]),
            None => method.reply(&[]),
        },

        ("GetAll", _) => {
            let mut all = BTreeMap::new();

            for name in [
                "RegisteredStatusNotifierItems",
                "IsStatusNotifierHostRegistered",
                "ProtocolVersion",
            ] {
                if let Some(value) = property(name) {
                    all.insert(String::from(name), value);
                }
            }

            method.reply(&[Argument::Map(all)]);
        }

        _ => method.reply(&[]),
    }
}

fn property(name: &str) -> Option<Argument> {
    match name {
        "RegisteredStatusNotifierItems" => {
            let tray = Tray::read();

            let items = tray.items.iter().map(|item| item.registered.clone());

            Some(Argument::TextList(items.collect()))
        }

        // the shell is the host, and it is running if anyone is asking
        "IsStatusNotifierHostRegistered" => Some(Argument::Bool(true)),

        "ProtocolVersion" => Some(Argument::Int(0)),

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_path_means_the_caller() {
        let (name, path) = address("/org/ayatana/NotificationItem/steam", ":1.42");

        assert_eq!(name, ":1.42");
        assert_eq!(path, "/org/ayatana/NotificationItem/steam");
    }

    #[test]
    fn a_bare_name_uses_the_usual_path() {
        let (name, path) = address("org.kde.StatusNotifierItem-1-1", ":1.42");

        assert_eq!(name, "org.kde.StatusNotifierItem-1-1");
        assert_eq!(path, DEFAULT_PATH);
    }

    #[test]
    fn a_name_joined_with_its_path_is_split() {
        let (name, path) = address(":1.7/org/example/Item", ":1.42");

        assert_eq!(name, ":1.7");
        assert_eq!(path, "/org/example/Item");
    }

    #[test]
    fn items_are_named_the_way_other_hosts_expect() {
        assert_eq!(
            registered("org.example.Item", DEFAULT_PATH),
            "org.example.Item"
        );
        assert_eq!(
            registered(":1.42", "/org/ayatana/Item"),
            ":1.42/org/ayatana/Item"
        );
    }
}
