//! midscope core: everything that talks to Riot, with no UI. No injection and
//! no game memory reads; only the Riot Client's local API and Riot's servers.

pub mod api;
pub mod config;
pub mod content;
pub mod game;
pub mod iconmatch;
pub mod layout;
pub mod lockfile;
pub mod rank;
pub mod scan;
pub mod update;
