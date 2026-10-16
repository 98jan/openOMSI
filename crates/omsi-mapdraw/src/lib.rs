//! The maps' own picture, shared by the game's navigator and city map: the palettes of the
//! light and the dark look (`style`), and the map's layers (`omsi_map::layers`, read off the
//! map by `omsi_scene::mapgen`) as triangles (`MapData`).

pub mod style;
mod mesh;

pub use mesh::{GroundLook, MapData};
