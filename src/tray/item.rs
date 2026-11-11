use std::path::{Path, PathBuf};

use amane::{Argument, Bus, Value};

use super::menu::MenuEntry;
use super::{icon, menu};

pub(super) const INTERFACE: &str = "org.kde.StatusNotifierItem";

// libdbusmenu's way of saying the item has no menu
const NO_MENU: &str = "/NO_DBUSMENU";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrayStatus {
    // idle, so most bars hide it
    Passive,

    #[default]
    Active,

    // wants to be noticed, and shows its attention icon
    NeedsAttention,
}

// one program's icon in the tray
#[derive(Debug, Clone, PartialEq)]
pub struct TrayItem {
    pub(super) id: u32,

    // how it registered, which is also how other tray hosts name it
    pub(super) registered: String,

    // its unique bus name, like ":1.42", which stays the same while it runs
    pub(super) owner: String,

    pub(super) path: String,

    pub(super) menu_path: String,

    app_id: String,
    title: String,
    status: TrayStatus,
    icon: Option<PathBuf>,
    attention_icon: Option<PathBuf>,
    overlay_icon: Option<PathBuf>,
    tooltip_title: String,
    tooltip_text: String,
    is_menu: bool,
    menu: Vec<MenuEntry>,
}

impl TrayItem {
    // what the Tray functions take
    pub fn id(&self) -> u32 {
        self.id
    }

    // the program's own name for the item, like "steam"
    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn status(&self) -> TrayStatus {
        self.status
    }

    // the image to draw, the attention one while the item needs attention; none when it has no icon
    pub fn icon_path(&self) -> Option<&Path> {
        if self.status == TrayStatus::NeedsAttention
            && let Some(attention) = &self.attention_icon
        {
            return Some(attention);
        }

        self.icon.as_deref()
    }

    // a small badge to draw over the icon, like an unread count
    pub fn overlay_icon_path(&self) -> Option<&Path> {
        self.overlay_icon.as_deref()
    }

    pub fn tooltip_title(&self) -> &str {
        &self.tooltip_title
    }

    pub fn tooltip_text(&self) -> &str {
        &self.tooltip_text
    }

    // true when a click should open the menu, because the item has nothing else to do
    pub fn is_menu(&self) -> bool {
        self.is_menu
    }

    // empty when the item has no menu of its own
    pub fn menu(&self) -> &[MenuEntry] {
        &self.menu
    }

    // empty until refresh() reads it
    pub(super) fn new(id: u32, registered: String, owner: String, path: String) -> Self {
        Self {
            id,
            registered,
            owner,
            path,
            menu_path: String::new(),
            app_id: String::new(),
            title: String::new(),
            status: TrayStatus::default(),
            icon: None,
            attention_icon: None,
            overlay_icon: None,
            tooltip_title: String::new(),
            tooltip_text: String::new(),
            is_menu: false,
            menu: Vec::new(),
        }
    }

    // every file this item made, so they can be removed when it goes
    pub(super) fn icons(&self) -> [Option<&Path>; 3] {
        [
            self.icon.as_deref(),
            self.attention_icon.as_deref(),
            self.overlay_icon.as_deref(),
        ]
    }

    // reads every property again; an item that does not answer stays as it was
    pub(super) fn refresh(&mut self) {
        let arguments = [Argument::from(INTERFACE)];

        let properties = Bus::session().call(
            &self.owner,
            &self.path,
            "org.freedesktop.DBus.Properties",
            "GetAll",
            &arguments,
        );

        if properties == Value::Nothing {
            return;
        }

        self.read(&properties);
    }

    pub(super) fn reload_menu(&mut self) {
        self.menu = if self.menu_path.is_empty() {
            Vec::new()
        } else {
            menu::read(&self.owner, &self.menu_path)
        };
    }

    fn read(&mut self, properties: &Value) {
        self.app_id = String::from(properties.get("Id").text());
        self.title = String::from(properties.get("Title").text());
        self.status = TrayStatus::from_name(properties.get("Status").text());
        self.is_menu = properties.get("ItemIsMenu").bool();

        let theme = properties.get("IconThemePath").text();

        self.icon = self.find_icon(properties, "Icon", theme, "icon");
        self.attention_icon = self.find_icon(properties, "AttentionIcon", theme, "attention");
        self.overlay_icon = self.find_icon(properties, "OverlayIcon", theme, "overlay");

        // (icon name, icon pixmaps, title, text)
        let tooltip = properties.get("ToolTip").list();

        if let [_, _, title, text] = tooltip {
            self.tooltip_title = String::from(title.text());
            self.tooltip_text = String::from(text.text());
        } else {
            self.tooltip_title.clear();
            self.tooltip_text.clear();
        }

        let menu_path = menu_path(properties.get("Menu").text());

        if menu_path != self.menu_path {
            self.menu_path = String::from(menu_path);

            self.reload_menu();
        }
    }

    // properties are named IconName and IconPixmap, AttentionIconName and so on
    fn find_icon(
        &self,
        properties: &Value,
        base: &str,
        theme: &str,
        slot: &str,
    ) -> Option<PathBuf> {
        icon::find(
            properties.get(&format!("{base}Name")).text(),
            theme,
            properties.get(&format!("{base}Pixmap")),
            self.id,
            slot,
        )
    }
}

impl TrayStatus {
    fn from_name(name: &str) -> Self {
        match name {
            "Passive" => Self::Passive,
            "NeedsAttention" => Self::NeedsAttention,
            _ => Self::Active,
        }
    }
}

// empty when the item has no menu
fn menu_path(path: &str) -> &str {
    if path == NO_MENU || path == "/" {
        ""
    } else {
        path
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn item() -> TrayItem {
        TrayItem::new(
            1,
            String::from(":1.5"),
            String::from(":1.5"),
            String::from("/item"),
        )
    }

    fn properties(entries: &[(&str, Value)]) -> Value {
        let map: BTreeMap<String, Value> = entries
            .iter()
            .map(|(name, value)| (String::from(*name), value.clone()))
            .collect();

        Value::Map(map)
    }

    fn text(value: &str) -> Value {
        Value::Text(String::from(value))
    }

    #[test]
    fn reads_the_plain_properties() {
        let mut item = item();

        item.read(&properties(&[
            ("Id", text("steam")),
            ("Title", text("Steam")),
            ("Status", text("NeedsAttention")),
            ("ItemIsMenu", Value::Bool(true)),
            (
                "ToolTip",
                Value::List(vec![
                    text(""),
                    Value::List(vec![]),
                    text("Steam"),
                    text("2 downloads"),
                ]),
            ),
        ]));

        assert_eq!(item.app_id(), "steam");
        assert_eq!(item.title(), "Steam");
        assert_eq!(item.status(), TrayStatus::NeedsAttention);
        assert!(item.is_menu());
        assert_eq!(item.tooltip_title(), "Steam");
        assert_eq!(item.tooltip_text(), "2 downloads");
    }

    #[test]
    fn an_unknown_status_counts_as_active() {
        assert_eq!(TrayStatus::from_name("Passive"), TrayStatus::Passive);
        assert_eq!(TrayStatus::from_name("Active"), TrayStatus::Active);
        assert_eq!(TrayStatus::from_name(""), TrayStatus::Active);
        assert_eq!(TrayStatus::from_name("Other"), TrayStatus::Active);
    }

    #[test]
    fn a_missing_tooltip_clears_the_old_one() {
        let mut item = item();

        item.tooltip_title = String::from("old");
        item.tooltip_text = String::from("old");

        item.read(&properties(&[]));

        assert_eq!(item.tooltip_title(), "");
        assert_eq!(item.tooltip_text(), "");
    }

    #[test]
    fn menu_paths_that_mean_no_menu_are_empty() {
        assert_eq!(menu_path("/NO_DBUSMENU"), "");
        assert_eq!(menu_path("/"), "");
        assert_eq!(menu_path(""), "");
        assert_eq!(menu_path("/MenuBar"), "/MenuBar");
    }

    #[test]
    fn the_attention_icon_shows_only_while_attention_is_needed() {
        let mut item = item();

        item.icon = Some(PathBuf::from("/icons/normal.png"));
        item.attention_icon = Some(PathBuf::from("/icons/attention.png"));

        assert_eq!(item.icon_path(), Some(Path::new("/icons/normal.png")));

        item.status = TrayStatus::NeedsAttention;

        assert_eq!(item.icon_path(), Some(Path::new("/icons/attention.png")));

        item.attention_icon = None;

        assert_eq!(item.icon_path(), Some(Path::new("/icons/normal.png")));
    }

    #[test]
    fn lists_the_files_it_made() {
        let mut item = item();

        assert!(item.icons().iter().all(Option::is_none));

        item.overlay_icon = Some(PathBuf::from("/icons/badge.png"));

        assert_eq!(item.icons()[2], Some(Path::new("/icons/badge.png")));
    }
}
