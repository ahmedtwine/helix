use anyhow::Result;

fn main() -> Result<()> {
    let exit_code = main_impl()?;
    std::process::exit(exit_code);
}

#[tokio::main]
async fn main_impl() -> Result<i32> {
    helix_studio::install();
    helix_term::entry::run(helix_studio::config::load).await
}
