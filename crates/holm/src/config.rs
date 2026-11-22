//! The config file, TOML with `version = 1` at the top.
//!
//! Holm reads an older version with one migration step per version. A file with no `version`
//! is version 0. Keys that Holm does not know stay in the file, and each gets a log line.
//! A bad value gives the default for that key. A file Holm can't read gives the defaults,
//! and Holm never writes over it. Holm also never writes over a file with a newer version.

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use gpui_kit::Context;
use holm_core::model::Order;
use holm_core::{FolderPath, NotePath, TagName};
use holm_editor::WritingHelp;
use holm_markdown::Color;
use toml_edit::{Array, DocumentMut, Item, Table, Value};

use crate::scale::Scale;
use crate::text;
use crate::text_width::TextWidthSetting;

const VERSION: i64 = 1;

/// The step at index `i` takes a file from version `i` to version `i + 1`.
const MIGRATIONS: [fn(&mut DocumentMut); 1] = [from_0];

const MAX_CUSTOM_COLORS: usize = 3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AppearanceSetting {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Settings {
    /// `None` until the user picks a folder.
    pub(crate) notes_folder: Option<PathBuf>,
    pub(crate) theme: AppearanceSetting,
    /// On top of the desktop scale, in the file as a percent.
    pub(crate) scale: Scale,
    pub(crate) text_width: TextWidthSetting,
    pub(crate) note_order: Order,
    pub(crate) writing_help: WritingHelp,
    /// The newest first.
    pub(crate) custom_colors: Vec<Color>,
    /// False after "Do not show again" in the delete dialog.
    pub(crate) ask_before_delete: bool,
    pub(crate) sidebar: SidebarSettings,
    /// Open last.
    pub(crate) last_note: Option<NotePath>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SidebarSettings {
    /// In pixels.
    pub(crate) width: u32,
    pub(crate) hidden: bool,
    pub(crate) view: SavedSidebarView,
}

impl Default for SidebarSettings {
    fn default() -> Self {
        Self {
            width: 250,
            hidden: false,
            view: SavedSidebarView {
                list: true,
                item: SidebarItem::All,
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SavedSidebarView {
    /// The note list of `item`, or else the tree with `item` selected.
    pub(crate) list: bool,
    pub(crate) item: SidebarItem,
}

/// An item of the tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SidebarItem {
    All,
    /// A folder that is not the root.
    Folder(FolderPath),
    Tag(TagName),
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            notes_folder: None,
            theme: AppearanceSetting::default(),
            scale: Scale::default(),
            text_width: TextWidthSetting::default(),
            note_order: Order::default(),
            writing_help: WritingHelp::default(),
            custom_colors: Vec::new(),
            ask_before_delete: true,
            sidebar: SidebarSettings::default(),
            last_note: None,
        }
    }
}

#[derive(Debug)]
pub(crate) struct Config {
    path: PathBuf,
    document: DocumentMut,
    settings: Settings,
    health: Health,
    save_failed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Health {
    Writable,
    Broken,
    Newer,
}

impl Config {
    /// A missing file gives the defaults.
    pub(crate) fn load(path: PathBuf) -> Self {
        let (document, health) = match fs::read_to_string(&path) {
            Ok(text) => parse(&path, &text),
            Err(error) if error.kind() == io::ErrorKind::NotFound => (DocumentMut::new(), Health::Writable),
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "cannot read the config file");
                (DocumentMut::new(), Health::Broken)
            }
        };
        let settings = if health == Health::Broken {
            Settings::default()
        } else {
            read(&document)
        };
        Self {
            path,
            document,
            settings,
            health,
            save_failed: false,
        }
    }

    pub(crate) fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Holm can't read the file, or the last save failed. A good save ends it.
    pub(crate) fn problem(&self) -> Option<&'static str> {
        (self.health == Health::Broken || self.save_failed).then_some(text::CONFIG_WRITE)
    }

    /// Applies `change` to the settings, writes them as [`Config::save`] does, and notifies the
    /// observers. Settings that stay the same are not written.
    pub(crate) fn change(&mut self, change: impl FnOnce(&mut Settings), cx: &mut Context<Self>) {
        let mut settings = self.settings.clone();
        change(&mut settings);
        if settings != self.settings {
            self.save(settings);
            cx.notify();
        }
    }

    /// It does not write a broken file or a file with a newer version. A folder path that is not
    /// UTF-8 stays out of the file.
    fn save(&mut self, settings: Settings) {
        self.settings = settings;
        if self.health != Health::Writable {
            tracing::info!(path = %self.path.display(), "the config file stays as it is");
            return;
        }
        write(&mut self.document, &self.settings);
        let written = self.write_file();
        if let Err(error) = &written {
            tracing::warn!(%error, path = %self.path.display(), "cannot write the config file");
        }
        self.save_failed = written.is_err();
    }

    fn write_file(&self) -> io::Result<()> {
        let dir = self.path.parent().unwrap_or(Path::new("."));
        fs::create_dir_all(dir)?;
        let mut file = tempfile::NamedTempFile::new_in(dir)?;
        file.write_all(self.document.to_string().as_bytes())?;
        file.persist(&self.path).map_err(|e| e.error)?;
        Ok(())
    }
}

fn parse(path: &Path, text: &str) -> (DocumentMut, Health) {
    let mut document = match text.parse::<DocumentMut>() {
        Ok(document) => document,
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "the config file is not TOML");
            return (DocumentMut::new(), Health::Broken);
        }
    };
    let version = match document.get("version") {
        None => 0,
        Some(item) => match item.as_integer() {
            Some(version) if version >= 0 => version,
            _ => {
                tracing::warn!(path = %path.display(), "the config version is not a number");
                return (DocumentMut::new(), Health::Broken);
            }
        },
    };
    if version > VERSION {
        tracing::warn!(
            path = %path.display(), version,
            "the config file is from a newer Holm, so Holm does not change it"
        );
        return (document, Health::Newer);
    }
    for step in MIGRATIONS.iter().skip(usize::try_from(version).unwrap_or(0)) {
        step(&mut document);
    }
    set(&mut document, "version", Value::from(VERSION));
    document.sort_values_by(|a, _, b, _| (a.get() != "version").cmp(&(b.get() != "version")));
    (document, Health::Writable)
}

/// A file from before `version` existed. Its keys are the same as in version 1.
fn from_0(_: &mut DocumentMut) {}

fn read(document: &DocumentMut) -> Settings {
    let mut settings = Settings::default();
    for (key, item) in document.iter() {
        match key {
            "version" => {}
            "notes_folder" => {
                if let Some(path) = get(key, item, Item::as_str) {
                    settings.notes_folder = Some(PathBuf::from(path));
                }
            }
            "theme" => {
                if let Some(theme) = get(key, item, |item| match item.as_str()? {
                    "system" => Some(AppearanceSetting::System),
                    "light" => Some(AppearanceSetting::Light),
                    "dark" => Some(AppearanceSetting::Dark),
                    _ => None,
                }) {
                    settings.theme = theme;
                }
            }
            "scale" => {
                if let Some(percent) = get(key, item, Item::as_integer) {
                    settings.scale = Scale::from_percent(percent);
                }
            }
            "note_order" => {
                if let Some(order) = get(key, item, |item| match item.as_str()? {
                    "last_change" => Some(Order::LastChange),
                    "title" => Some(Order::Title),
                    _ => None,
                }) {
                    settings.note_order = order;
                }
            }
            "ask_before_delete" => {
                if let Some(ask) = get(key, item, Item::as_bool) {
                    settings.ask_before_delete = ask;
                }
            }
            "custom_colors" => {
                if let Some(colors) = get(key, item, |item| {
                    item.as_array()?
                        .iter()
                        .map(|color| Color::from_hex(color.as_str()?))
                        .collect::<Option<Vec<_>>>()
                }) {
                    settings.custom_colors = colors.into_iter().take(MAX_CUSTOM_COLORS).collect();
                }
            }
            "writing_help" => {
                if let Some(table) = get(key, item, Item::as_table_like) {
                    read_writing_help(table, &mut settings.writing_help);
                }
            }
            "text_width" => {
                if let Some(table) = get(key, item, Item::as_table_like) {
                    read_text_width(table, &mut settings.text_width);
                }
            }
            "last_note" => {
                if let Some(path) = get(key, item, Item::as_str) {
                    settings.last_note = NotePath::new(path);
                }
            }
            "sidebar" => {
                if let Some(table) = get(key, item, Item::as_table_like) {
                    read_sidebar(table, &mut settings.sidebar);
                }
            }
            _ => unknown(key),
        }
    }
    settings
}

fn read_writing_help(table: &dyn toml_edit::TableLike, help: &mut WritingHelp) {
    for (key, item) in table.iter() {
        let field = match key {
            "format_keys" => &mut help.format_keys,
            "continue_lists" => &mut help.continue_lists,
            "indent_lists" => &mut help.indent_lists,
            _ => {
                unknown(&format!("writing_help.{key}"));
                continue;
            }
        };
        if let Some(on) = get(key, item, Item::as_bool) {
            *field = on;
        }
    }
}

fn read_text_width(table: &dyn toml_edit::TableLike, width: &mut TextWidthSetting) {
    for (key, item) in table.iter() {
        match key {
            "mode" => {
                if let Some(limited) = get(key, item, |item| match item.as_str()? {
                    "full" => Some(false),
                    "limited" => Some(true),
                    _ => None,
                }) {
                    width.limited = limited;
                }
            }
            "chars" => {
                let mut read = *width;
                if get(key, item, |item| read.set_chars(item.as_integer()?).then_some(())).is_some() {
                    *width = read;
                }
            }
            _ => unknown(&format!("text_width.{key}")),
        }
    }
}

fn read_sidebar(table: &dyn toml_edit::TableLike, sidebar: &mut SidebarSettings) {
    for (key, value) in table.iter() {
        match key {
            "width" => {
                if let Some(width) = get(key, value, |value| u32::try_from(value.as_integer()?).ok()) {
                    sidebar.width = width;
                }
            }
            "hidden" => {
                if let Some(hidden) = get(key, value, Item::as_bool) {
                    sidebar.hidden = hidden;
                }
            }
            "view" => {
                if let Some(list) = get(key, value, |value| match value.as_str()? {
                    "tree" => Some(false),
                    "list" => Some(true),
                    _ => None,
                }) {
                    sidebar.view.list = list;
                }
            }
            "item" => {
                if let Some(read) = get(key, value, |value| item_from(value.as_str()?)) {
                    sidebar.view.item = read;
                }
            }
            _ => unknown(&format!("sidebar.{key}")),
        }
    }
}

/// `all`, `folder:<path>` or `tag:<name>`.
fn item_from(text: &str) -> Option<SidebarItem> {
    match text.split_once(':') {
        None if text == "all" => Some(SidebarItem::All),
        Some(("folder", path)) if !path.is_empty() => FolderPath::new(path).map(SidebarItem::Folder),
        Some(("tag", name)) => TagName::new(name).map(SidebarItem::Tag),
        _ => None,
    }
}

/// A folder path that is not UTF-8 gives `all`.
fn item_text(item: &SidebarItem) -> String {
    match item {
        SidebarItem::Folder(path) => match AsRef::<Path>::as_ref(path).to_str() {
            Some(path) => format!("folder:{path}"),
            None => "all".to_owned(),
        },
        SidebarItem::Tag(name) => format!("tag:{name}"),
        SidebarItem::All => "all".to_owned(),
    }
}

/// A bad value gives a log line.
fn get<'a, T>(key: &str, item: &'a Item, value: impl FnOnce(&'a Item) -> Option<T>) -> Option<T> {
    let value = value(item);
    if value.is_none() {
        tracing::warn!(key, "the config value is bad, so Holm uses the default");
    }
    value
}

fn unknown(key: &str) {
    tracing::info!(key, "the config file has a key that Holm does not know");
}

fn write(document: &mut DocumentMut, settings: &Settings) {
    if document.is_empty() {
        set(document, "version", Value::from(VERSION));
    }
    match settings.notes_folder.as_deref().map(Path::to_str) {
        Some(Some(path)) => set(document, "notes_folder", Value::from(path)),
        Some(None) => {
            tracing::warn!("the notes folder path is not UTF-8, so the file leaves it out");
        }
        None => {}
    }
    let theme = match settings.theme {
        AppearanceSetting::System => "system",
        AppearanceSetting::Light => "light",
        AppearanceSetting::Dark => "dark",
    };
    set(document, "theme", Value::from(theme));
    set(document, "scale", Value::from(i64::from(settings.scale.percent())));
    let order = match settings.note_order {
        Order::LastChange => "last_change",
        Order::Title => "title",
    };
    set(document, "note_order", Value::from(order));
    set(document, "ask_before_delete", Value::from(settings.ask_before_delete));
    let colors: Array = settings.custom_colors.iter().map(ToString::to_string).collect();
    set(document, "custom_colors", Value::from(colors));
    match settings
        .last_note
        .as_ref()
        .map(|path| path.as_ref().to_str().map(str::to_owned))
    {
        Some(Some(path)) => set(document, "last_note", Value::from(path)),
        Some(None) | None => {
            document.remove("last_note");
        }
    }

    if !document.get("writing_help").is_some_and(Item::is_table) {
        document.insert("writing_help", Item::Table(Table::new()));
    }
    if let Some(table) = document.get_mut("writing_help").and_then(Item::as_table_mut) {
        let help = settings.writing_help;
        set(table, "format_keys", Value::from(help.format_keys));
        set(table, "continue_lists", Value::from(help.continue_lists));
        set(table, "indent_lists", Value::from(help.indent_lists));
    }

    if !document.get("text_width").is_some_and(Item::is_table) {
        document.insert("text_width", Item::Table(Table::new()));
    }
    if let Some(table) = document.get_mut("text_width").and_then(Item::as_table_mut) {
        let width = settings.text_width;
        let mode = if width.limited { "limited" } else { "full" };
        set(table, "mode", Value::from(mode));
        set(table, "chars", Value::from(i64::from(width.chars())));
    }

    if !document.get("sidebar").is_some_and(Item::is_table) {
        document.insert("sidebar", Item::Table(Table::new()));
    }
    if let Some(table) = document.get_mut("sidebar").and_then(Item::as_table_mut) {
        let sidebar = &settings.sidebar;
        set(table, "width", Value::from(i64::from(sidebar.width)));
        set(table, "hidden", Value::from(sidebar.hidden));
        let view = if sidebar.view.list { "list" } else { "tree" };
        set(table, "view", Value::from(view));
        set(table, "item", Value::from(item_text(&sidebar.view.item)));
    }
}

/// Keeps the comments around the old value.
fn set(table: &mut Table, key: &str, value: Value) {
    match table.get_mut(key).and_then(Item::as_value_mut) {
        Some(old) => {
            let decor = old.decor().clone();
            *old = value;
            *old.decor_mut() = decor;
        }
        None => {
            table.insert(key, Item::Value(value));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_paths::{folder_path, tag_name};

    fn load(text: &str) -> (tempfile::TempDir, Config) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, text).unwrap();
        let config = Config::load(path);
        (dir, config)
    }

    fn file(config: &Config) -> String {
        fs::read_to_string(&config.path).unwrap()
    }

    #[test]
    fn a_missing_file_gives_the_defaults_and_save_writes_version_1_first() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::load(dir.path().join("holm").join("config.toml"));
        assert_eq!(config.settings(), &Settings::default());
        assert_eq!(config.problem(), None);

        let settings = Settings {
            notes_folder: Some(PathBuf::from("/home/me/Notes")),
            theme: AppearanceSetting::Dark,
            scale: Scale::from_percent(125),
            text_width: limited_width(120),
            note_order: Order::Title,
            writing_help: WritingHelp {
                indent_lists: false,
                ..WritingHelp::default()
            },
            custom_colors: vec![Color {
                r: 0xc4,
                g: 0x55,
                b: 0x3a,
            }],
            ask_before_delete: false,
            sidebar: SidebarSettings {
                width: 310,
                hidden: true,
                view: SavedSidebarView {
                    list: false,
                    item: SidebarItem::Tag(tag_name("work/holm")),
                },
            },
            last_note: NotePath::new("work/Plans.md"),
        };
        config.save(settings.clone());

        assert!(file(&config).starts_with("version = 1\n"));
        assert_eq!(Config::load(config.path.clone()).settings(), &settings);
    }

    #[test]
    fn a_file_with_no_version_is_migrated_to_version_1() {
        let (_dir, mut config) = load("# My settings\ntheme = \"dark\"\n");
        assert_eq!(config.settings().theme, AppearanceSetting::Dark);

        config.save(config.settings().clone());

        let text = file(&config);
        assert!(text.starts_with("version = 1\n"), "{text}");
        assert!(text.contains("# My settings\ntheme = \"dark\"\n"), "{text}");
    }

    #[test]
    fn unknown_keys_stay_in_the_file() {
        let (_dir, mut config) =
            load("version = 1\nfuture = \"yes\"\n\n[writing_help]\nsmart_quotes = true\n\n[plugins]\nx = 1\n");

        let mut settings = config.settings().clone();
        settings.theme = AppearanceSetting::Light;
        config.save(settings);

        let text = file(&config);
        for line in [
            "future = \"yes\"",
            "smart_quotes = true",
            "[plugins]",
            "x = 1",
            "theme = \"light\"",
        ] {
            assert!(text.contains(line), "{line} is missing in:\n{text}");
        }
    }

    #[test]
    fn the_sidebar_list_and_its_item_come_back() {
        let (_dir, config) = load("version = 1\n[sidebar]\nview = \"list\"\nitem = \"folder:work\"\n");
        assert_eq!(
            config.settings().sidebar.view,
            SavedSidebarView {
                list: true,
                item: SidebarItem::Folder(folder_path("work")),
            }
        );
        let (_dir, config) = load("version = 1\n[sidebar]\nview = \"tree\"\nitem = \"bad\"\n");
        assert_eq!(
            config.settings().sidebar.view,
            SavedSidebarView {
                list: false,
                item: SidebarItem::All,
            }
        );
    }

    #[test]
    fn a_bad_value_gives_the_default_for_that_key() {
        let (_dir, config) = load("version = 1\ntheme = \"blue\"\nnote_order = \"title\"\n");
        assert_eq!(config.settings().theme, AppearanceSetting::System);
        assert_eq!(config.settings().note_order, Order::Title);
        assert_eq!(config.problem(), None);
    }

    #[test]
    fn the_scale_is_a_percent_and_a_value_that_is_not_a_step_reads_as_100() {
        let (_dir, mut config) = load("version = 1\nscale = 150\n");
        assert_eq!(config.settings().scale.percent(), 150);

        let mut settings = config.settings().clone();
        settings.scale = Scale::from_percent(80);
        config.save(settings);
        assert!(file(&config).contains("scale = 80\n"), "{}", file(&config));

        for bad in ["scale = 120", "scale = 0", "scale = \"big\"", "scale = 1.5"] {
            let (_dir, config) = load(&format!("version = 1\n{bad}\n"));
            assert_eq!(config.settings().scale.percent(), 100, "{bad}");
        }
    }

    fn limited_width(chars: i64) -> TextWidthSetting {
        let mut width = TextWidthSetting::default();
        assert!(width.set_chars(chars));
        width.limited = true;
        width
    }

    #[test]
    fn the_text_width_keeps_its_mode_and_width_apart() {
        let (_dir, mut config) = load("version = 1\n[text_width]\nmode = \"limited\"\nchars = 120\n");
        assert_eq!(config.settings().text_width, limited_width(120));

        let mut settings = config.settings().clone();
        settings.text_width.limited = false;
        config.save(settings);
        let text = file(&config);
        assert!(text.contains("[text_width]\nmode = \"full\"\nchars = 120\n"), "{text}");
        let width = Config::load(config.path.clone()).settings().text_width;
        assert!(!width.limited);
        assert_eq!(width.chars(), 120);
    }

    #[test]
    fn a_text_width_out_of_the_range_reads_as_80() {
        for bad in [
            "chars = 20",
            "chars = 201",
            "chars = -80",
            "chars = \"wide\"",
            "chars = 99.5",
        ] {
            let (_dir, config) = load(&format!("version = 1\n[text_width]\nmode = \"limited\"\n{bad}\n"));
            assert_eq!(config.settings().text_width, limited_width(80), "{bad}");
        }
        let (_dir, config) = load("version = 1\n[text_width]\nmode = \"wide\"\n");
        assert_eq!(config.settings().text_width, TextWidthSetting::default());
    }

    #[test]
    fn a_broken_file_gives_the_defaults_and_stays_as_it_is() {
        let text = "version = 1\ntheme = \"dark\n";
        let (_dir, mut config) = load(text);
        assert_eq!(config.problem(), Some(text::CONFIG_WRITE));
        assert_eq!(config.settings(), &Settings::default());

        let settings = Settings {
            theme: AppearanceSetting::Light,
            ..Settings::default()
        };
        config.save(settings.clone());

        assert_eq!(config.settings(), &settings);
        assert_eq!(file(&config), text);
    }

    #[test]
    fn a_newer_version_is_read_but_not_written() {
        let text = "version = 2\ntheme = \"dark\"\n";
        let (_dir, mut config) = load(text);
        assert_eq!(config.problem(), None);
        assert_eq!(config.settings().theme, AppearanceSetting::Dark);

        config.save(Settings::default());
        assert_eq!(file(&config), text);
    }

    #[test]
    fn a_failed_save_is_a_problem_until_a_save_works() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("holm");
        let mut config = Config::load(folder.join("config.toml"));
        fs::write(&folder, "").unwrap();

        config.save(Settings::default());
        assert_eq!(config.problem(), Some(text::CONFIG_WRITE));

        fs::remove_file(&folder).unwrap();
        config.save(Settings::default());
        assert_eq!(config.problem(), None);
    }
}
