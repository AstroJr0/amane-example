use amane::{Argument, Bus, Value};

pub(super) const INTERFACE: &str = "com.canonical.dbusmenu";

// a menu deeper than this is a broken program, not a menu
const MAX_DEPTH: usize = 8;

// what a menu entry is, and for the toggles, whether it is on
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuKind {
    Normal,      // normal?
    Separator,   // a line between groups of entries, with no label
    Check(bool), // an entry that is on or off, shown with a tick
    Radio(bool), // one of a group where only one is on, shown with a dot
}

// one line of a tray item's own menu
#[derive(Debug, Clone, PartialEq)]
pub struct MenuEntry {
    id: i32,
    label: String,
    kind: MenuKind,
    enabled: bool,
    children: Vec<MenuEntry>,
}

impl MenuEntry {
    // what Tray::click_menu takes
    pub fn id(&self) -> i32 {
        self.id
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn kind(&self) -> MenuKind {
        self.kind
    }

    // a disabled entry is shown greyed out, and clicking it does nothing
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    // empty unless the entry opens a submenu
    pub fn children(&self) -> &[MenuEntry] {
        &self.children
    }
}

/*
 * the program fills a menu in when it is about to be shown, so it is asked
 * first; the layout comes back as (revision, (id, properties, children)),
 * and the root is only the holder of the entries
 */
pub fn read(owner: &str, path: &str) -> Vec<MenuEntry> {
    let bus = Bus::session();

    bus.call(owner, path, INTERFACE, "AboutToShow", &[Argument::from(0)]);

    let layout = bus.call(
        owner,
        path,
        INTERFACE,
        "GetLayout",
        &[
            Argument::from(0),
            Argument::from(-1),
            Argument::from(Vec::<String>::new()),
        ],
    );

    let [_revision, root] = layout.list() else {
        return Vec::new();
    };

    children(root, 0)
}

// the program does the rest, like running the command or flipping the tick
pub fn click(owner: &str, path: &str, id: i32) {
    let arguments = [
        Argument::from(id),
        Argument::from("clicked"),
        Argument::Variant(Box::new(Argument::from(0))),
        Argument::from(0_u32),
    ];

    Bus::session().call(owner, path, INTERFACE, "Event", &arguments);
}

// a node is (id, properties, children), and each child sits in a variant
fn children(node: &Value, depth: usize) -> Vec<MenuEntry> {
    let [_id, _properties, children] = node.list() else {
        return Vec::new();
    };

    children
        .list()
        .iter()
        .filter_map(|child| entry(child, depth))
        .collect()
}

fn entry(node: &Value, depth: usize) -> Option<MenuEntry> {
    let [id, properties, _children] = node.list() else {
        return None;
    };

    if !flag(properties, "visible", true) {
        return None;
    }

    let kind = if properties.get("type").text() == "separator" {
        MenuKind::Separator
    } else {
        toggle(properties)
    };

    Some(MenuEntry {
        id: id.number() as i32,
        label: label(properties.get("label").text()),
        kind,
        enabled: flag(properties, "enabled", true),
        children: if depth < MAX_DEPTH {
            children(node, depth + 1)
        } else {
            Vec::new()
        },
    })
}

// a missing property means the default, which is not the same as false
fn flag(properties: &Value, name: &str, default: bool) -> bool {
    match properties.get(name) {
        Value::Bool(value) => *value,
        _ => default,
    }
}

// toggle-state is 1 for on, 0 for off, and -1 when it is neither
fn toggle(properties: &Value) -> MenuKind {
    let on = properties.get("toggle-state").number() == 1.0;

    match properties.get("toggle-type").text() {
        "checkmark" => MenuKind::Check(on),
        "radio" => MenuKind::Radio(on),
        _ => MenuKind::Normal,
    }
}

// a single underscore marks the key to press, and a double one is an underscore
fn label(text: &str) -> String {
    let mut label = String::with_capacity(text.len());

    let mut characters = text.chars().peekable();

    while let Some(character) = characters.next() {
        if character != '_' {
            label.push(character);

            continue;
        }

        if characters.peek() == Some(&'_') {
            label.push('_');

            characters.next();
        }
    }

    label
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn number(value: i32) -> Value {
        Value::Number(f64::from(value))
    }

    fn text(value: &str) -> Value {
        Value::Text(String::from(value))
    }

    fn properties(entries: &[(&str, Value)]) -> Value {
        let map: BTreeMap<String, Value> = entries
            .iter()
            .map(|(name, value)| (String::from(*name), value.clone()))
            .collect();

        Value::Map(map)
    }

    fn node(id: i32, entries: &[(&str, Value)], children: Vec<Value>) -> Value {
        Value::List(vec![number(id), properties(entries), Value::List(children)])
    }

    fn layout(children: Vec<Value>) -> Value {
        Value::List(vec![number(1), node(0, &[], children)])
    }

    fn root_entries(layout: &Value) -> Vec<MenuEntry> {
        let [_revision, root] = layout.list() else {
            panic!("a layout has a revision and a root");
        };

        children(root, 0)
    }

    #[test]
    fn reads_labels_and_ids() {
        let layout = layout(vec![
            node(1, &[("label", text("Open"))], vec![]),
            node(2, &[("label", text("Quit"))], vec![]),
        ]);

        let entries = root_entries(&layout);

        assert_eq!(entries.len(), 2);
        assert_eq!((entries[0].id(), entries[0].label()), (1, "Open"));
        assert_eq!((entries[1].id(), entries[1].label()), (2, "Quit"));
    }

    #[test]
    fn missing_flags_mean_enabled_and_visible() {
        let layout = layout(vec![
            node(1, &[("label", text("Plain"))], vec![]),
            node(
                2,
                &[("label", text("Off")), ("enabled", Value::Bool(false))],
                vec![],
            ),
            node(
                3,
                &[("label", text("Hidden")), ("visible", Value::Bool(false))],
                vec![],
            ),
        ]);

        let entries = root_entries(&layout);

        assert_eq!(entries.len(), 2);
        assert!(entries[0].enabled());
        assert!(!entries[1].enabled());
    }

    #[test]
    fn reads_separators_and_toggles() {
        let layout = layout(vec![
            node(1, &[("type", text("separator"))], vec![]),
            node(
                2,
                &[
                    ("label", text("Mute")),
                    ("toggle-type", text("checkmark")),
                    ("toggle-state", number(1)),
                ],
                vec![],
            ),
            node(
                3,
                &[
                    ("label", text("Low")),
                    ("toggle-type", text("radio")),
                    ("toggle-state", number(0)),
                ],
                vec![],
            ),
        ]);

        let kinds: Vec<MenuKind> = root_entries(&layout).iter().map(MenuEntry::kind).collect();

        assert_eq!(
            kinds,
            [
                MenuKind::Separator,
                MenuKind::Check(true),
                MenuKind::Radio(false)
            ]
        );
    }

    #[test]
    fn reads_submenus() {
        let submenu = node(
            5,
            &[("label", text("More"))],
            vec![node(6, &[("label", text("Inner"))], vec![])],
        );

        let entries = root_entries(&layout(vec![submenu]));

        assert_eq!(entries[0].children().len(), 1);
        assert_eq!(entries[0].children()[0].label(), "Inner");
    }

    #[test]
    fn stops_at_the_depth_limit() {
        let mut deepest = node(0, &[("label", text("end"))], vec![]);

        for id in 1..=20 {
            deepest = node(id, &[("label", text("level"))], vec![deepest]);
        }

        let mut entry = &root_entries(&layout(vec![deepest]))[0];
        let mut levels = 1;

        while let Some(child) = entry.children().first() {
            entry = child;
            levels += 1;
        }

        assert_eq!(levels, MAX_DEPTH + 1);
    }

    #[test]
    fn an_answer_that_is_not_a_layout_gives_nothing() {
        assert!(children(&Value::Nothing, 0).is_empty());
        assert!(children(&text("bad"), 0).is_empty());
    }

    #[test]
    fn strips_mnemonics_but_keeps_literal_underscores() {
        assert_eq!(label("_File"), "File");
        assert_eq!(label("Save _As"), "Save As");
        assert_eq!(label("snake__case"), "snake_case");
        assert_eq!(label("plain"), "plain");
    }
}
