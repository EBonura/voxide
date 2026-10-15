//! Render pieces of music from seamless loops.
//!
//! A game that streams one stream at a time (the PS1 CD drive decodes a single
//! XA channel) cannot overlap two loops at run time, so the overlap is mixed
//! offline: each piece is a seeded sequence of whole loops, each played one to
//! three times, consecutive loops joined by an equal-power crossfade centred
//! on the loop boundary. Every loop is seamless, so it is read as a circle: the
//! outgoing loop runs on into its own next lap during the fade and the incoming
//! one starts in its own tail, and both are at the right place when the
//! boundary passes. The piece is then resampled with a Kaiser-windowed sinc and
//! dithered to 16 bits.

pub mod mix;
pub mod plan;
pub mod resample;
pub mod rng;
pub mod wav;
