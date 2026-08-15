//! CLI entry. clap subcommands arrive in phase 5a.

fn main() {
    println!(
        "vic3-cli {} (plan {}) — see docs/usage.md",
        env!("CARGO_PKG_VERSION"),
        vic3_plan::version()
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn bin_compiles() {
        assert!(!env!("CARGO_PKG_VERSION").is_empty());
    }
}
