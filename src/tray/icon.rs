use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{self, Command, Stdio};
use std::sync::LazyLock;
use std::thread;

use amane::Value;

const FOLDER: &str = "tray-icons";

// how deep an icon theme goes: theme, size, category, and a little spare
const MAX_DEPTH: usize = 5;

/*
 * the best file for every icon name on the system, read once. walking the
 * themes takes seconds, so the tray starts it early, and the first item
 * that needs a name may have to wait for it
 */
static ICONS: LazyLock<HashMap<String, PathBuf>> = LazyLock::new(index);

pub fn warm_up() {
    thread::spawn(|| {
        LazyLock::force(&ICONS);
    });
}

/*
 * the picture for one icon of an item, where the slot says which one: the
 * normal, the attention or the overlay icon. the spec asks for the icon name
 * first, looked up in the item's own theme folder and then the system's, and
 * for the pixmap only when the name finds nothing
 */
pub fn find(
    name: &str,
    theme_folder: &str,
    pixmaps: &Value,
    item: u32,
    slot: &str,
) -> Option<PathBuf> {
    from_name(name, theme_folder).or_else(|| from_pixmaps(pixmaps, item, slot))
}

// only pictures saved here are removed, never an icon the system or the item owns
pub fn remove(icon: &Path) {
    if icon.parent() == Some(&folder()) {
        let _ = fs::remove_file(icon);
    }
}

fn from_name(name: &str, theme_folder: &str) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }

    // some programs give the file itself
    if name.starts_with('/') {
        return Some(PathBuf::from(name)).filter(|path| path.is_file());
    }

    // programs that ship icons of their own put them in a folder, loose
    if !theme_folder.is_empty() {
        for extension in ["svg", "png"] {
            let path = Path::new(theme_folder).join(format!("{name}.{extension}"));

            if path.is_file() {
                return Some(path);
            }
        }
    }

    ICONS.get(name).cloned()
}

// the biggest drawing is shrunk sharply, and a small one would only be stretched
fn from_pixmaps(pixmaps: &Value, item: u32, slot: &str) -> Option<PathBuf> {
    let (width, height, bytes) = biggest(pixmaps)?;

    let pixels = rgba(&bytes);

    let mut hasher = DefaultHasher::new();

    (width, height, &pixels).hash(&mut hasher);

    // named by what it holds, so an icon that did not change keeps its file
    let name = format!(
        "{}-{item}-{slot}-{:016x}.png",
        process::id(),
        hasher.finish()
    );

    let path = folder().join(name);

    if path.is_file() {
        return Some(path);
    }

    save(&path, width, height, &pixels)?;

    Some(path)
}

// each pixmap is (width, height, bytes), and one that is cut short is skipped
fn biggest(pixmaps: &Value) -> Option<(u32, u32, Vec<u8>)> {
    let mut biggest: Option<(u32, u32, Vec<u8>)> = None;

    for pixmap in pixmaps.list() {
        let [width, height, bytes] = pixmap.list() else {
            continue;
        };

        let (width, height) = (width.number() as u32, height.number() as u32);

        let bytes: Vec<u8> = bytes
            .list()
            .iter()
            .map(|byte| byte.number() as u8)
            .collect();

        let whole = width as usize * height as usize * 4 == bytes.len();

        let larger = biggest.as_ref().is_none_or(|(best, ..)| width > *best);

        if width > 0 && height > 0 && whole && larger {
            biggest = Some((width, height, bytes));
        }
    }

    biggest
}

// the bus sends each pixel as alpha, red, green, blue, and png wants red, green, blue, alpha
fn rgba(argb: &[u8]) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(argb.len());

    for pixel in argb.chunks_exact(4) {
        pixels.extend_from_slice(&[pixel[1], pixel[2], pixel[3], pixel[0]]);
    }

    pixels
}

// amane draws pictures from files, so a pixmap becomes a png; stored deflate
// blocks keep that to a few lines, and an icon is too small to need more
fn save(path: &Path, width: u32, height: u32, pixels: &[u8]) -> Option<()> {
    let row = width as usize * 4;

    let mut raw = Vec::with_capacity(pixels.len() + height as usize);

    for line in pixels.chunks_exact(row) {
        // filter type none
        raw.push(0);
        raw.extend_from_slice(line);
    }

    // zlib header, then blocks of at most 65535 bytes, the last one marked
    let mut data = vec![0x78, 0x01];
    let mut blocks = raw.chunks(usize::from(u16::MAX)).peekable();

    while let Some(block) = blocks.next() {
        let length = block.len() as u16;

        data.push(u8::from(blocks.peek().is_none()));
        data.extend_from_slice(&length.to_le_bytes());
        data.extend_from_slice(&(!length).to_le_bytes());
        data.extend_from_slice(block);
    }

    data.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut header = Vec::with_capacity(13);

    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    // 8 bits, rgba, and the plain methods
    header.extend_from_slice(&[8, 6, 0, 0, 0]);

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();

    chunk(&mut png, b"IHDR", &header);
    chunk(&mut png, b"IDAT", &data);
    chunk(&mut png, b"IEND", &[]);

    fs::create_dir_all(path.parent()?).ok()?;
    fs::write(path, png).ok()
}

fn chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    png.extend_from_slice(&(data.len() as u32).to_be_bytes());
    png.extend_from_slice(kind);
    png.extend_from_slice(data);

    let mut crc = 0xffff_ffff_u32;

    for &byte in kind.iter().chain(data) {
        crc ^= u32::from(byte);

        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (crc & 1).wrapping_neg());
        }
    }

    png.extend_from_slice(&(!crc).to_be_bytes());
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1_u32, 0_u32);

    for &byte in data {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }

    (b << 16) | a
}

fn folder() -> PathBuf {
    let runtime_directory =
        env::var_os("XDG_RUNTIME_DIR").unwrap_or_else(|| env::temp_dir().into());

    PathBuf::from(runtime_directory).join(FOLDER)
}

/*
 * the icon theme the user chose comes first, then hicolor, which is where
 * programs put icons of their own, then every other theme; inside a theme the
 * scalable icon wins, and else the biggest. loose icons come last
 */
fn index() -> HashMap<String, PathBuf> {
    let roots = icon_folders();

    let themes = ranked_themes(&roots);

    // the name, with the theme's rank and the quality of the file
    let mut best: HashMap<String, (usize, u32, PathBuf)> = HashMap::new();

    for root in &roots {
        for (rank, theme) in themes.iter().enumerate() {
            walk(&root.join(theme), 0, &mut |file| {
                add(&mut best, rank, file);
            });
        }

        // files straight in the folder belong to no theme, and at this depth the walk goes no further down
        walk(root, MAX_DEPTH, &mut |file| {
            add(&mut best, themes.len(), file)
        });
    }

    for pixmaps in pixmap_folders() {
        walk(&pixmaps, 0, &mut |file| add(&mut best, themes.len(), file));
    }

    best.into_iter()
        .map(|(name, (_, _, file))| (name, file))
        .collect()
}

// a better theme always wins, and in the same theme a better file does
fn add(best: &mut HashMap<String, (usize, u32, PathBuf)>, rank: usize, file: &Path) {
    let Some(name) = icon_name(file) else {
        return;
    };

    let quality = quality(file);

    match best.get_mut(name) {
        Some((held, held_quality, held_file)) => {
            if rank < *held || (rank == *held && quality > *held_quality) {
                (*held, *held_quality, *held_file) = (rank, quality, file.to_path_buf());
            }
        }

        None => {
            best.insert(String::from(name), (rank, quality, file.to_path_buf()));
        }
    }
}

// only svg and png are drawn, so the name is the file's without either
fn icon_name(file: &Path) -> Option<&str> {
    match file.extension().and_then(OsStr::to_str) {
        Some("svg" | "png") => file.file_stem().and_then(OsStr::to_str),
        _ => None,
    }
}

// scalable beats everything, then comes the size the folder names, like 48x48 or 48
fn quality(file: &Path) -> u32 {
    if file.extension() == Some(OsStr::new("svg")) {
        return u32::MAX;
    }

    file.components()
        .filter_map(|part| part.as_os_str().to_str())
        .filter_map(|part| part.split('x').next()?.parse::<u32>().ok())
        .max()
        .unwrap_or(0)
}

// calls the visitor for every file under a folder, a symlinked one too
fn walk(folder: &Path, depth: usize, visit: &mut impl FnMut(&Path)) {
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();

        // a link says nothing about what it points at
        let is_folder = match entry.file_type() {
            Ok(kind) if kind.is_symlink() => path.is_dir(),
            Ok(kind) => kind.is_dir(),
            Err(_) => false,
        };

        if !is_folder {
            visit(&path);
        } else if depth < MAX_DEPTH {
            walk(&path, depth + 1, visit);
        }
    }
}

fn ranked_themes(roots: &[PathBuf]) -> Vec<String> {
    let mut all: Vec<String> = roots
        .iter()
        .filter_map(|root| fs::read_dir(root).ok())
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();

    all.sort();
    all.dedup();

    let mut ranked = Vec::with_capacity(all.len());

    for first in [user_theme(), Some(String::from("hicolor"))]
        .into_iter()
        .flatten()
    {
        if let Some(at) = all.iter().position(|theme| *theme == first) {
            ranked.push(all.remove(at));
        }
    }

    ranked.extend(all);

    ranked
}

// the user's own folders first, so their icons win over the system's
fn icon_folders() -> Vec<PathBuf> {
    let mut folders = Vec::new();

    if let Some(home) = env::var_os("HOME") {
        folders.push(PathBuf::from(home).join(".icons"));
    }

    folders.extend(
        data_folders()
            .into_iter()
            .map(|folder| folder.join("icons")),
    );

    folders
}

fn pixmap_folders() -> Vec<PathBuf> {
    data_folders()
        .into_iter()
        .map(|folder| folder.join("pixmaps"))
        .collect()
}

// the XDG data folders, the user's first
fn data_folders() -> Vec<PathBuf> {
    let home = env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));

    let system = env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|dirs| !dirs.is_empty())
        .unwrap_or_else(|| String::from("/usr/local/share:/usr/share"));

    home.into_iter()
        .chain(env::split_paths(&system))
        .filter(|path| path.is_absolute())
        .collect()
}

// gtk's own settings file, and when it says nothing, the desktop's settings
fn user_theme() -> Option<String> {
    gtk_theme().or_else(desktop_theme)
}

fn gtk_theme() -> Option<String> {
    let config = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;

    ["gtk-4.0", "gtk-3.0"].iter().find_map(|folder| {
        let settings = fs::read_to_string(config.join(folder).join("settings.ini")).ok()?;

        theme_in_settings(&settings)
    })
}

fn theme_in_settings(settings: &str) -> Option<String> {
    settings.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;

        (key.trim() == "gtk-icon-theme-name").then(|| String::from(value.trim()))
    })
}

// gsettings prints the name in quotes, like 'Adwaita'
fn desktop_theme() -> Option<String> {
    let output = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "icon-theme"])
        .stderr(Stdio::null())
        .output()
        .ok()?;

    let theme = String::from_utf8(output.stdout).ok()?;

    let theme = theme.trim().trim_matches('\'');

    (output.status.success() && !theme.is_empty()).then(|| String::from(theme))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn number(value: u32) -> Value {
        Value::Number(f64::from(value))
    }

    fn pixmap(width: u32, height: u32, bytes: &[u8]) -> Value {
        Value::List(vec![
            number(width),
            number(height),
            Value::List(bytes.iter().map(|byte| number(u32::from(*byte))).collect()),
        ])
    }

    #[test]
    fn puts_alpha_last() {
        assert_eq!(
            rgba(&[255, 1, 2, 3, 128, 4, 5, 6]),
            [1, 2, 3, 255, 4, 5, 6, 128]
        );
    }

    #[test]
    fn picks_the_widest_pixmap() {
        let small = pixmap(1, 1, &[255, 0, 0, 0]);
        let large = pixmap(2, 1, &[255, 0, 0, 0, 255, 1, 1, 1]);

        let pixmaps = Value::List(vec![small, large.clone(), pixmap(1, 1, &[255, 9, 9, 9])]);

        let (width, height, bytes) = biggest(&pixmaps).unwrap();

        assert_eq!((width, height), (2, 1));
        assert_eq!(bytes.len(), 8);
    }

    #[test]
    fn skips_a_pixmap_with_the_wrong_amount_of_bytes() {
        let cut_short = pixmap(2, 2, &[255, 0, 0, 0]);

        assert!(biggest(&Value::List(vec![cut_short])).is_none());
    }

    #[test]
    fn skips_anything_that_is_not_a_pixmap() {
        assert!(biggest(&Value::Nothing).is_none());
        assert!(biggest(&Value::List(vec![Value::Bool(true)])).is_none());
    }

    #[test]
    fn a_missing_name_finds_nothing() {
        assert!(from_name("", "").is_none());
        assert!(from_name("/no/such/icon.png", "").is_none());
    }

    #[test]
    fn an_icon_path_that_exists_is_used_as_it_is() {
        let file = env::temp_dir().join(format!("amane-tray-icon-{}.png", process::id()));

        fs::write(&file, b"").unwrap();

        let found = from_name(file.to_str().unwrap(), "");

        fs::remove_file(&file).unwrap();

        assert_eq!(found, Some(file));
    }

    #[test]
    fn saves_a_pixmap_once_and_names_it_by_its_content() {
        let pixmaps = Value::List(vec![pixmap(1, 1, &[255, 10, 20, 30])]);

        let first = from_pixmaps(&pixmaps, 7, "icon").unwrap();
        let second = from_pixmaps(&pixmaps, 7, "icon").unwrap();

        let other = Value::List(vec![pixmap(1, 1, &[255, 11, 20, 30])]);
        let changed = from_pixmaps(&other, 7, "icon").unwrap();

        assert_eq!(first, second);
        assert_ne!(first, changed);
        assert!(first.is_file());

        remove(&first);
        remove(&changed);

        assert!(!first.exists());
    }

    fn scratch(name: &str) -> PathBuf {
        let folder = env::temp_dir().join(format!("tray-icon-{name}-{}", process::id()));

        let _ = fs::remove_dir_all(&folder);

        fs::create_dir_all(&folder).unwrap();

        folder
    }

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"").unwrap();
    }

    #[test]
    fn only_svg_and_png_are_icons() {
        assert_eq!(icon_name(Path::new("/a/b/firefox.png")), Some("firefox"));
        assert_eq!(icon_name(Path::new("/a/b/firefox.svg")), Some("firefox"));
        assert_eq!(icon_name(Path::new("/a/b/firefox.xpm")), None);
        assert_eq!(icon_name(Path::new("/a/b/index.theme")), None);
    }

    #[test]
    fn scalable_beats_size_and_size_beats_nothing() {
        let svg = quality(Path::new("/i/T/scalable/apps/a.svg"));
        let big = quality(Path::new("/i/T/128x128/apps/a.png"));
        let small = quality(Path::new("/i/T/16x16/apps/a.png"));
        let sizeless = quality(Path::new("/i/T/apps/a.png"));

        assert!(svg > big && big > small && small > sizeless);
        assert_eq!(quality(Path::new("/i/T/apps/48/a.png")), 48);
    }

    #[test]
    fn a_better_theme_wins_and_inside_one_the_better_file() {
        let mut best = HashMap::new();

        add(&mut best, 1, Path::new("/i/Other/48x48/apps/a.png"));
        add(&mut best, 0, Path::new("/i/Mine/16x16/apps/a.png"));
        add(&mut best, 0, Path::new("/i/Mine/64x64/apps/a.png"));
        add(&mut best, 0, Path::new("/i/Mine/32x32/apps/a.png"));
        add(&mut best, 2, Path::new("/i/Late/scalable/apps/a.svg"));

        assert_eq!(best["a"].2, Path::new("/i/Mine/64x64/apps/a.png"));
    }

    #[test]
    fn walks_every_file_but_only_to_the_depth() {
        let root = scratch("walk");

        touch(&root.join("top.png"));
        touch(&root.join("a/b/deep.png"));
        touch(&root.join("a/b/c/d/e/f/g/too-deep.png"));

        let mut seen = Vec::new();

        walk(&root, 0, &mut |file| seen.push(file.to_path_buf()));

        assert!(seen.contains(&root.join("top.png")));
        assert!(seen.contains(&root.join("a/b/deep.png")));
        assert!(!seen.contains(&root.join("a/b/c/d/e/f/g/too-deep.png")));

        let mut loose = Vec::new();

        walk(&root, MAX_DEPTH, &mut |file| loose.push(file.to_path_buf()));

        assert_eq!(loose, [root.join("top.png")]);

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn themes_are_ranked_mine_then_hicolor_then_the_rest() {
        let root = scratch("rank");

        for theme in ["Zeta", "hicolor", "Alpha", "Mine"] {
            fs::create_dir_all(root.join(theme)).unwrap();
        }

        fs::write(root.join("not-a-theme.png"), b"").unwrap();

        // no user theme is set up in a test, so only hicolor is moved to the front
        let ranked = ranked_themes(&[root.clone()]);

        assert_eq!(ranked[0], "hicolor");
        assert_eq!(ranked[1..], ["Alpha", "Mine", "Zeta"]);

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn finds_an_icon_loose_in_the_items_own_folder() {
        let folder = scratch("own");

        touch(&folder.join("tray-mono.png"));

        let found = from_name("tray-mono", folder.to_str().unwrap());

        assert_eq!(found, Some(folder.join("tray-mono.png")));

        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn reads_the_icon_theme_from_gtk_settings() {
        let settings = "[Settings]\ngtk-theme-name=Adw\ngtk-icon-theme-name = Papirus-Dark\n";

        assert_eq!(theme_in_settings(settings).as_deref(), Some("Papirus-Dark"));
        assert_eq!(theme_in_settings("[Settings]\n"), None);
    }
}
