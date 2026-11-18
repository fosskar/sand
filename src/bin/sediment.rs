//! sediment, the name sand had before 0.6.0; kept so existing setups keep working.

fn main() -> anyhow::Result<()> {
    sand::deprecated("the sediment executable", "sand");
    sand::cli::run()
}
