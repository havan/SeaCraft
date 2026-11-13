//! The fonts compiled into the program: DejaVu Sans and DejaVu Sans Mono 2.37, unmodified, from
//! `assets/fonts`, with their licence. The desktop GUI sets its text in them, and PDF exports embed
//! a subset of DejaVu Sans ([`crate::ttf`], [`crate::pdf`]). They are not under the GPL; `NOTICE`
//! carries their copyright lines.

/// DejaVu Sans 2.37, the whole file.
pub const DEJAVU_SANS: &[u8] = include_bytes!("../assets/fonts/DejaVuSans.ttf");

/// DejaVu Sans Mono 2.37, the whole file.
pub const DEJAVU_SANS_MONO: &[u8] = include_bytes!("../assets/fonts/DejaVuSansMono.ttf");

/// The licence both fonts are distributed under.
pub const DEJAVU_LICENCE: &str = include_str!("../assets/fonts/LICENSE-DejaVu.txt");
