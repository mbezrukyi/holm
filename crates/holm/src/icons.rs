//! Holm's own icons: small SVG files inside the binary, drawn in one color by GPUI.

use std::borrow::Cow;

use gpui_kit::{AssetSource, Hsla, IntoElement, SharedString, Styled, Svg, px, svg};

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
    Search,
    Menu,
}

impl Icon {
    const EVERY: [Self; 8] = [
        Self::All,
        Self::Folder,
        Self::FolderOpen,
        Self::Tag,
        Self::Tags,
        Self::TagsOpen,
        Self::Search,
        Self::Menu,
    ];

    /// The asset path and the SVG file.
    fn file(self) -> (&'static str, &'static [u8]) {
        match self {
            Self::All => ("icons/all.svg", include_bytes!("../icons/all.svg")),
            Self::Folder => ("icons/folder.svg", include_bytes!("../icons/folder.svg")),
            Self::FolderOpen => ("icons/folder-open.svg", include_bytes!("../icons/folder-open.svg")),
            Self::Tag => ("icons/tag.svg", include_bytes!("../icons/tag.svg")),
            Self::Tags => ("icons/tags.svg", include_bytes!("../icons/tags.svg")),
            Self::TagsOpen => ("icons/tags-open.svg", include_bytes!("../icons/tags-open.svg")),
            Self::Search => ("icons/search.svg", include_bytes!("../icons/search.svg")),
            Self::Menu => ("icons/menu.svg", include_bytes!("../icons/menu.svg")),
        }
    }

    pub(crate) fn view(self, color: Hsla) -> impl IntoElement {
        let icon: Svg = svg().path(self.file().0).size(px(16.)).flex_none();
        icon.text_color(color)
    }
}

pub(crate) struct Icons;

impl AssetSource for Icons {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        Ok(Icon::EVERY
            .iter()
            .map(|icon| icon.file())
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| Cow::Borrowed(bytes)))
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        Ok(Icon::EVERY
            .iter()
            .map(|icon| icon.file().0)
            .filter(|name| name.starts_with(path))
            .map(SharedString::from)
            .collect())
    }
}

#[cfg(test)]
mod tests;
