//! Migration numbering (#66): the two engines' migration directories are held
//! to one rule. No database, no feature: it runs on every `cargo test`.

mod check;
mod repository;
mod rules;
