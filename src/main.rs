use clap::{Parser, Subcommand};
use nixposer::{
    composer::{Composer, build_repository},
    model::ComposerPackageIdentifier,
    store::Store,
};
use url::Url;

#[derive(Parser, Debug)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Nix binary cache store URL.
    /// Currently only Local Binary Cache Store is supported.
    #[arg(long, global = true, default_value = "file://.")]
    store: Url,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Fetch a package and save it in the Nix binary cache.
    Memo {
        /// package identifier in format: foo/bar@1.2.3
        package: ComposerPackageIdentifier,

        /// Composer repository
        #[arg(long, default_value = "https://repo.packagist.org")]
        repo: Url,
    },
    /// Index the Nix binary cache, build a Composer v2 compatible repository and save it in the binary cache.
    Index {},
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let cli = Cli::parse();

    let store = Store::new(&cli.store)?;
    store.init()?;

    match cli.command {
        Command::Memo { package, repo } => {
            let composer = Composer::new(&repo)?;
            let (meta, data) = composer.fetch_package(&package).await?;
            let path = store.insert_package(&meta, &data)?;
            println!("{path}");
        }
        Command::Index {} => {
            let index = store.index()?;
            let data = build_repository(&index)?;
            let path = store.insert_repository(&data)?;
            println!("{path}");
        }
    }

    Ok(())
}
