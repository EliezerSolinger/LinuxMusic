//! The application's pages: Composition and Mixing, two pages switched by the tabs on top (Cmd/Ctrl+1, +2,
//! Ctrl+Tab), with the transport bar over both (user's decision, 19/11).

pub mod composition;
pub mod desk;
pub mod mixing;
pub mod transport;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Composition,
    Mixing,
}

impl Page {
    pub fn title(self) -> &'static str {
        match self {
            Page::Composition => "Composition",
            Page::Mixing => "Mixing",
        }
    }
}
