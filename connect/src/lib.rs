#![warn(missing_docs)]
#![doc=include_str!("../README.md")]

#[macro_use]
extern crate log;

use librespot_core as core;
use librespot_playback as playback;
use librespot_protocol as protocol;

mod capability_debug;
mod context_resolver;
mod mix_debug;
mod model;
mod shuffle_vec;
mod spirc;
mod state;

pub use model::*;
pub use spirc::*;
pub use state::*;
