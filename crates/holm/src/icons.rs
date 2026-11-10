//! Holm's own icons: small SVG files inside the binary, drawn in one color by GPUI.

use std::borrow::Cow;

use gpui_kit::{AssetSource, Hsla, IntoElement, SharedString, Styled, Svg, px, svg};

/// The icon files, by asset path.
const FILES: [(&str, &[u8]); 6] = [
    ("icons/all.svg", include_bytes!("../icons/all.svg")),
    ("icons/folder.svg", include_bytes!("../icons/folder.svg")),
    (
        "icons/folder-open.svg",
        include_bytes!("../icons/folder-open.svg"),
    ),
    ("icons/tag.svg", include_bytes!("../icons/tag.svg")),
    ("icons/tags.svg", include_bytes!("../icons/tags.svg")),
    (
        "icons/tags-open.svg",
        include_bytes!("../icons/tags-open.svg"),
    ),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Icon {
    All,
    Folder,
    FolderOpen,
    /// A tag with no child tags.
    Tag,
    /// A closed tag with child tags.
    Tags,
    TagsOpen,
}

impl Icon {
    fn path(self) -> &'static str {
        match self {
            Self::All => "icons/all.svg",
            Self::Folder => "icons/folder.svg",
            Self::FolderOpen => "icons/folder-open.svg",
            Self::Tag => "icons/tag.svg",
            Self::Tags => "icons/tags.svg",
            Self::TagsOpen => "icons/tags-open.svg",
        }
    }

    /// The icon at 16 px in `color`.
    pub(crate) fn view(self, color: Hsla) -> impl IntoElement {
        let icon: Svg = svg().path(self.path()).size(px(16.)).flex_none();
        icon.text_color(color)
    }
}

/// The asset source of the app. It holds only the icons.
pub(crate) struct Icons;

impl AssetSource for Icons {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        Ok(FILES
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        Ok(FILES
            .iter()
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| (*name).into())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_has_a_file() {
        for icon in [
            Icon::All,
            Icon::Folder,
            Icon::FolderOpen,
            Icon::Tag,
            Icon::Tags,
            Icon::TagsOpen,
        ] {
            let file = Icons.load(icon.path()).ok().flatten();
            assert!(
                file.is_some_and(|bytes| bytes.starts_with(b"<svg")),
                "{icon:?}"
            );
        }
    }
}
