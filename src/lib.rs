//! Validate the structure and references of a documentation collection.
//!
//! The library does not print, execute document code, or start a service.
//!
//! Call the library from a Rust test:
//!
//! ```no_run
//! use perfect_doc::{Config, report, validate};
//! use std::path::PathBuf;
//!
//! let result = validate(&[PathBuf::from(".")], &Config::default())?;
//! assert!(result.is_valid(), "{}", report::human(&result));
//! # Ok::<(), perfect_doc::ScanError>(())
//! ```

mod assets;
mod collection;
pub mod config;
mod headings;
mod html_reader;
mod includes;
mod links;
mod markdown_reader;
mod metadata;
pub mod model;
mod online;
pub mod report;
mod scan;

pub use config::Config;
pub use model::{Diagnostic, Outcome, Report, Severity};
pub use scan::{ScanError, validate, validate_with_cancel};

/// Return all supported rules in stable order.
pub fn rules() -> Vec<model::RuleDefinition> {
    let mut rules = Vec::new();
    rules.extend_from_slice(scan::RULES);
    rules.extend_from_slice(metadata::RULES);
    rules.extend_from_slice(markdown_reader::RULES);
    rules.extend_from_slice(html_reader::RULES);
    rules.extend_from_slice(links::RULES);
    rules.extend_from_slice(collection::RULES);
    rules.extend_from_slice(includes::RULES);
    rules.extend_from_slice(assets::RULES);
    rules.extend_from_slice(online::RULES);
    rules.sort_by_key(|rule| rule.id);
    rules
}
