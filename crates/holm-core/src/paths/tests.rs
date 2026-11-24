use super::*;

fn folder(path: &str) -> FolderPath {
    FolderPath::new(path).unwrap()
}

fn note(path: &str) -> NotePath {
    NotePath::new(path).unwrap()
}

fn problem<T: fmt::Debug>(result: Result<T>) -> NameProblem {
    match result {
        Err(Error::BadName { problem, .. }) => problem,
        other => panic!("not a bad name: {other:?}"),
    }
}

#[test]
fn typed_names_give_each_name_problem() {
    let long = "a".repeat(MAX_NAME_BYTES - 2);
    assert_eq!(problem(Name::typed("  ")), NameProblem::Empty);
    assert_eq!(problem(Name::typed("a/b")), NameProblem::Slash);
    assert_eq!(problem(Name::typed("..")), NameProblem::Dot);
    assert_eq!(problem(Name::typed("a\0b")), NameProblem::ZeroByte);
    assert_eq!(problem(Name::typed(&"a".repeat(256))), NameProblem::TooLong);
    assert!(Name::typed(&long).is_ok());
    assert_eq!(problem(Name::title(&long)), NameProblem::TooLong);
    assert_eq!(problem(Name::title(".a")), NameProblem::Dot);
}

#[test]
fn typed_names_lose_the_spaces_around_them() {
    let work = folder("work");
    let title = Name::title(" Plans ").unwrap();
    assert_eq!(work.note(&title), note("work/Plans.md"));
    assert_eq!(work.child(&Name::typed(" 2026 ").unwrap()), folder("work/2026"));
    let attachments = Name::typed("attachments").unwrap();
    assert_eq!(FolderPath::root().child(&attachments), folder("attachments"));
}

#[test]
fn disk_names_skip_hidden_names_only() {
    let disk = |name: &str| Name::from_disk(OsStr::new(name));
    let work = folder("work");
    assert_eq!(work.child(&disk(" x ").unwrap()), folder("work/ x "));
    assert_eq!(work.child(&disk("attachments").unwrap()), folder("work/attachments"));
    assert_eq!(work.note_file(&disk("a.md").unwrap()), Some(note("work/a.md")));
    assert_eq!(work.note_file(&disk("a.txt").unwrap()), None);
    assert_eq!(disk(".a.md"), None);
    assert_eq!(disk(".git"), None);
    assert_eq!(disk(""), None);
    assert_eq!(disk(&"a".repeat(256)), None);
}

#[test]
fn a_folder_splits_into_its_folder_and_name() {
    let (parent, name) = folder("work/2026").split().unwrap();
    assert_eq!(parent, folder("work"));
    assert_eq!(parent.child(&name), folder("work/2026"));
    assert!(FolderPath::root().split().is_none());
}

#[test]
fn a_raw_path_splits_only_when_it_is_plain() {
    let split = |path: &str| {
        RawPath::new(path)
            .and_then(|path| FolderPath::split_raw(&path))
            .map(|(folder, name)| (folder, name.as_os_str().to_owned()))
    };
    assert_eq!(split("work/a.png"), Some((folder("work"), "a.png".into())));
    assert_eq!(split("a.md"), Some((FolderPath::root(), "a.md".into())));
    for raw in ["", "/a", "work/", ".git/x", "work/.a", "a//b", "./a"] {
        assert_eq!(split(raw), None, "{raw}");
    }
}
