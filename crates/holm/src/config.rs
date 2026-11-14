//! The config file, TOML with `version = 1` at the top.
//!
//! Holm reads an older version with one migration step per version. A file with no `version`
//! is version 0. Keys that Holm does not know stay in the file, and each gets a log line.
//! A bad value gives the default for that key. A file Holm can't read gives the defaults,
//! and Holm never writes over it. Holm also never writes over a file with a newer version.

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use holm_core::model::Order;
use holm_editor::WritingHelp;
use holm_markdown::Color;
use toml_edit::{Array, DocumentMut, Item, Table, Value};

use crate::error::AppError;
use crate::sidebar::{Item as SidebarItem, View};

const VERSION: i64 = 1;

/// The step at index `i` takes a file from version `i` to version `i + 1`.
const MIGRATIONS: [fn(&mut DocumentMut); 1] = [from_0];

/// The most custom colors the file keeps.
const CUSTOM_COLORS: usize = 3;

/// Light or dark mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Theme {
    /// The mode of the system.
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Settings {
    /// `None` until the user picks a folder.
    pub(crate) notes_folder: Option<PathBuf>,
    pub(crate) theme: Theme,
    pub(crate) note_order: Order,
    pub(crate) writing_help: WritingHelp,
    /// The last custom colors, the newest first.
    pub(crate) custom_colors: Vec<Color>,
    /// False after "Do not show again" in the delete dialog.
    pub(crate) ask_before_delete: bool,
    pub(crate) sidebar: SidebarSettings,
    /// The note that was open last, relative to the notes folder.
    pub(crate) last_note: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SidebarSettings {
    /// In pixels.
    pub(crate) width: u16,
    pub(crate) hidden: bool,
    pub(crate) view: View,
}

impl Default for SidebarSettings {
    fn default() -> Self {
        Self {
            width: 250,
            hidden: false,
            view: View::default(),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            notes_folder: None,
            theme: Theme::default(),
            note_order: Order::default(),
            writing_help: WritingHelp::default(),
            custom_colors: Vec::new(),
            ask_before_delete: true,
            sidebar: SidebarSettings::default(),
            last_note: None,
        }
    }
}

/// The config file and the settings in it.
#[derive(Debug)]
pub(crate) struct Config {
    path: PathBuf,
    document: DocumentMut,
    settings: Settings,
    state: State,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Writable,
    Broken,
    Newer,
}

impl Config {
    /// Reads the file at `path`. A missing file gives the defaults.
    pub(crate) fn load(path: PathBuf) -> Self {
        let (document, state) = match fs::read_to_string(&path) {
            Ok(text) => parse(&path, &text),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                (DocumentMut::new(), State::Writable)
            }
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "cannot read the config file");
                (DocumentMut::new(), State::Broken)
            }
        };
        let settings = if state == State::Broken {
            Settings::default()
        } else {
            read(&document)
        };
        Self {
            path,
            document,
            settings,
            state,
        }
    }

    pub(crate) fn settings(&self) -> &Settings {
        &self.settings
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Whether Holm could not read the file. The folder screen then tells why it asks.
    pub(crate) fn is_broken(&self) -> bool {
        self.state == State::Broken
    }

    /// Keeps `settings` and writes them to the file. It does not write a broken file or a file
    /// with a newer version. A folder path that is not UTF-8 stays out of the file.
    pub(crate) fn save(&mut self, settings: Settings) -> Result<(), AppError> {
        self.settings = settings;
        if self.state != State::Writable {
            tracing::info!(path = %self.path.display(), "the config file stays as it is");
            return Ok(());
        }
        write(&mut self.document, &self.settings);
        let error = |source| AppError::ConfigWrite {
            path: self.path.clone(),
            source,
        };
        let dir = self.path.parent().unwrap_or(Path::new("."));
        fs::create_dir_all(dir).map_err(error)?;
        let mut file = tempfile::NamedTempFile::new_in(dir).map_err(error)?;
        file.write_all(self.document.to_string().as_bytes())
            .map_err(error)?;
        file.persist(&self.path).map_err(|e| error(e.error))?;
        Ok(())
    }
}

fn parse(path: &Path, text: &str) -> (DocumentMut, State) {
    let mut document = match text.parse::<DocumentMut>() {
        Ok(document) => document,
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "the config file is not TOML");
            return (DocumentMut::new(), State::Broken);
        }
    };
    let version = match document.get("version") {
        None => 0,
        Some(item) => match item.as_integer() {
            Some(version) if version >= 0 => version,
            _ => {
                tracing::warn!(path = %path.display(), "the config version is not a number");
                return (DocumentMut::new(), State::Broken);
            }
        },
    };
    if version > VERSION {
        tracing::warn!(
            path = %path.display(), version,
            "the config file is from a newer Holm, so Holm does not change it"
        );
        return (document, State::Newer);
    }
    for step in MIGRATIONS
        .iter()
        .skip(usize::try_from(version).unwrap_or(0))
    {
        step(&mut document);
    }
    set(&mut document, "version", Value::from(VERSION));
    document.sort_values_by(|a, _, b, _| (a.get() != "version").cmp(&(b.get() != "version")));
    (document, State::Writable)
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
                    "system" => Some(Theme::System),
                    "light" => Some(Theme::Light),
                    "dark" => Some(Theme::Dark),
                    _ => None,
                }) {
                    settings.theme = theme;
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
                    settings.custom_colors = colors.into_iter().take(CUSTOM_COLORS).collect();
                }
            }
            "writing_help" => {
                if let Some(table) = get(key, item, Item::as_table_like) {
                    read_writing_help(table, &mut settings.writing_help);
                }
            }
            "last_note" => {
                if let Some(path) = get(key, item, Item::as_str) {
                    settings.last_note = Some(PathBuf::from(path));
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

fn read_sidebar(table: &dyn toml_edit::TableLike, sidebar: &mut SidebarSettings) {
    let mut list = matches!(sidebar.view, View::List(_));
    let mut item = sidebar.view.item().clone();
    for (key, value) in table.iter() {
        match key {
            "width" => {
                if let Some(width) =
                    get(key, value, |value| u16::try_from(value.as_integer()?).ok())
                {
                    sidebar.width = width;
                }
            }
            "hidden" => {
                if let Some(hidden) = get(key, value, Item::as_bool) {
                    sidebar.hidden = hidden;
                }
            }
            "view" => {
                if let Some(view) = get(key, value, |value| match value.as_str()? {
                    "tree" => Some(false),
                    "list" => Some(true),
                    _ => None,
                }) {
                    list = view;
                }
            }
            "item" => {
                if let Some(read) = get(key, value, |value| item_from(value.as_str()?)) {
                    item = read;
                }
            }
            _ => unknown(&format!("sidebar.{key}")),
        }
    }
    sidebar.view = if list {
        View::List(item)
    } else {
        View::Tree(item)
    };
}

/// `all`, `folder:<path>` or `tag:<name>`.
fn item_from(text: &str) -> Option<SidebarItem> {
    match text.split_once(':') {
        None if text == "all" => Some(SidebarItem::All),
        Some(("folder", path)) if !path.is_empty() => {
            Some(SidebarItem::Folder(PathBuf::from(path)))
        }
        Some(("tag", name)) if !name.is_empty() => Some(SidebarItem::Tag(name.to_owned())),
        _ => None,
    }
}

/// The text of `item` in the file. A folder path that is not UTF-8 gives `all`.
fn item_text(item: &SidebarItem) -> String {
    match item {
        SidebarItem::Folder(path) => match path.to_str() {
            Some(path) => format!("folder:{path}"),
            None => "all".to_owned(),
        },
        SidebarItem::Tag(name) => format!("tag:{name}"),
        SidebarItem::All => "all".to_owned(),
    }
}

/// The value of a known key, or a log line when the value is bad.
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
        Theme::System => "system",
        Theme::Light => "light",
        Theme::Dark => "dark",
    };
    set(document, "theme", Value::from(theme));
    let order = match settings.note_order {
        Order::LastChange => "last_change",
        Order::Title => "title",
    };
    set(document, "note_order", Value::from(order));
    set(
        document,
        "ask_before_delete",
        Value::from(settings.ask_before_delete),
    );
    let colors: Array = settings
        .custom_colors
        .iter()
        .map(ToString::to_string)
        .collect();
    set(document, "custom_colors", Value::from(colors));
    match settings.last_note.as_deref().map(Path::to_str) {
        Some(Some(path)) => set(document, "last_note", Value::from(path)),
        Some(None) | None => {
            document.remove("last_note");
        }
    }

    if !document.get("writing_help").is_some_and(Item::is_table) {
        document.insert("writing_help", Item::Table(Table::new()));
    }
    if let Some(table) = document
        .get_mut("writing_help")
        .and_then(Item::as_table_mut)
    {
        let help = settings.writing_help;
        set(table, "format_keys", Value::from(help.format_keys));
        set(table, "continue_lists", Value::from(help.continue_lists));
        set(table, "indent_lists", Value::from(help.indent_lists));
    }

    if !document.get("sidebar").is_some_and(Item::is_table) {
        document.insert("sidebar", Item::Table(Table::new()));
    }
    if let Some(table) = document.get_mut("sidebar").and_then(Item::as_table_mut) {
        let sidebar = &settings.sidebar;
        set(table, "width", Value::from(i64::from(sidebar.width)));
        set(table, "hidden", Value::from(sidebar.hidden));
        let view = match sidebar.view {
            View::Tree(_) => "tree",
            View::List(_) => "list",
        };
        set(table, "view", Value::from(view));
        set(table, "item", Value::from(item_text(sidebar.view.item())));
    }
}

/// Sets a value and keeps the comments around the old one.
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
        assert!(!config.is_broken());

        let settings = Settings {
            notes_folder: Some(PathBuf::from("/home/me/Notes")),
            theme: Theme::Dark,
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
                view: View::Tree(SidebarItem::Tag("work/holm".into())),
            },
            last_note: Some(PathBuf::from("work/Plans.md")),
        };
        config.save(settings.clone()).unwrap();

        assert!(file(&config).starts_with("version = 1\n"));
        assert_eq!(Config::load(config.path.clone()).settings(), &settings);
    }

    #[test]
    fn a_file_with_no_version_is_migrated_to_version_1() {
        let (_dir, mut config) = load("# My settings\ntheme = \"dark\"\n");
        assert_eq!(config.settings().theme, Theme::Dark);

        config.save(config.settings().clone()).unwrap();

        let text = file(&config);
        assert!(text.starts_with("version = 1\n"), "{text}");
        assert!(text.contains("# My settings\ntheme = \"dark\"\n"), "{text}");
    }

    #[test]
    fn unknown_keys_stay_in_the_file() {
        let (_dir, mut config) = load(
            "version = 1\nfuture = \"yes\"\n\n[writing_help]\nsmart_quotes = true\n\n[plugins]\nx = 1\n",
        );

        let mut settings = config.settings().clone();
        settings.theme = Theme::Light;
        config.save(settings).unwrap();

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
        let (_dir, config) =
            load("version = 1\n[sidebar]\nview = \"list\"\nitem = \"folder:work\"\n");
        assert_eq!(
            config.settings().sidebar.view,
            View::List(SidebarItem::Folder("work".into()))
        );
        let (_dir, config) = load("version = 1\n[sidebar]\nview = \"tree\"\nitem = \"bad\"\n");
        assert_eq!(config.settings().sidebar.view, View::Tree(SidebarItem::All));
    }

    #[test]
    fn a_bad_value_gives_the_default_for_that_key() {
        let (_dir, config) = load("version = 1\ntheme = \"blue\"\nnote_order = \"title\"\n");
        assert_eq!(config.settings().theme, Theme::System);
        assert_eq!(config.settings().note_order, Order::Title);
        assert!(!config.is_broken());
    }

    #[test]
    fn a_broken_file_gives_the_defaults_and_stays_as_it_is() {
        let text = "version = 1\ntheme = \"dark\n";
        let (_dir, mut config) = load(text);
        assert!(config.is_broken());
        assert_eq!(config.settings(), &Settings::default());

        let settings = Settings {
            theme: Theme::Light,
            ..Settings::default()
        };
        config.save(settings.clone()).unwrap();

        assert_eq!(config.settings(), &settings);
        assert_eq!(file(&config), text);
    }

    #[test]
    fn a_newer_version_is_read_but_not_written() {
        let text = "version = 2\ntheme = \"dark\"\n";
        let (_dir, mut config) = load(text);
        assert!(!config.is_broken());
        assert_eq!(config.settings().theme, Theme::Dark);

        config.save(Settings::default()).unwrap();
        assert_eq!(file(&config), text);
    }
}
