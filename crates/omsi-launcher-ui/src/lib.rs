//! The launcher's interface, apart from the game: the toolkit its pages are drawn with
//! (`ui`, `theme`) - flat and dark, every control custom, drawn with `omsi-ui` - which the
//! game's own menus (pause menu, photo mode, developer tools) use as well. The data side
//! (content lists, timetables, profiles, installs, running games) is `omsi-launcher-core`.

pub mod theme;
pub mod ui;
