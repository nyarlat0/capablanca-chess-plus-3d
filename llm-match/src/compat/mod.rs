//! Fairy-chess representation experiments and frontend compatibility.
//! Classic uses SAN and never loads these profile/rules files.
pub(crate) mod config;
mod match_support;
#[cfg(test)]
mod match_tests;
pub(crate) mod move_text;
pub(crate) mod profile;
pub(crate) mod representation;
#[cfg(test)]
mod representation_tests;
pub(crate) mod rules;
