//! Pure Rust numeric core for ICI analysis. No wasm-bindgen, no DOM.

pub mod decimate;
pub mod deriv;
pub mod derive;
pub mod optimal_window;
pub mod parse;
pub mod regress;
pub mod segment;
pub mod spline;
pub mod types;

pub fn placeholder_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_version_is_nonempty() {
        assert!(!placeholder_version().is_empty());
    }
}
