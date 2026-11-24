use holm_core::testing::{folder_path, tag_name};

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
