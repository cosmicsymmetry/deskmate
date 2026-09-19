//! The render case table: every card face x both mount orientations x the
//! data states chosen to exercise each face's visually distinct appearances.
//! `tests/golden.rs` pins the landscape outputs under `tests/golden/`; flipped
//! rows remain available to the physical harness. The physical framebuffer
//! diff drives both [`crate::cases::scene_cases`] and
//! [`crate::cases::face_scene_cases`], because a scene is the only thing shipping
//! firmware draws. This lives under `src/` rather
//! than `tests/` so integration tests and hardware examples can reach it as
//! `lvgl_sim::cases`.
//!
//! Every value a node binds comes from the closed vocabulary in
//! `firmware/main/core/scene_binding.h`: `time:`, `date` and the `timer.*`
//! family.

mod date_overflow;
mod faces;
mod node_kinds;

pub use date_overflow::{
    DATE_CONTENT_WIDTH, DATE_OVERFLOW_BODY_WIDTH, DATE_OVERFLOW_NOW_UNIX_SECONDS,
    DATE_OVERFLOW_UTC_OFFSET_MINUTES, date_overflow_text, date_truncation_scene_cases,
};
pub use faces::{TABULAR_0000, TABULAR_1135, face_scene_cases};
pub use node_kinds::{asset_font_scene_cases, scene_cases};

use crate::SimOrientation;

fn orientations() -> [(&'static str, SimOrientation); 2] {
    [
        ("landscape", SimOrientation::Landscape),
        ("flipped", SimOrientation::LandscapeFlipped),
    ]
}
