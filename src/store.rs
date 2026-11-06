use std::{collections::HashMap, fs, path::PathBuf};

use itertools::Itertools;
use nix_derivation::{
    CAHash, HashAlgorithm, StoreDir, StorePath, hash_bytes, nixbase32, store_path::build_ca_path,
};
use nix_narinfo::{Compression, Field, NarInfo};
use thiserror::Error;
use url::Url;

use crate::{model::ComposerPackageMetadata, tree::Tree};

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("Provided scheme is not supported.")]
    UsupportedSchema,

    #[error("Invalid Narinfo Extension.")]
    InvalideNarinfoExteions,

    #[error(transparent)]
    TreeError(#[from] crate::tree::TreeError),

    #[error(transparent)]
    StorePathError(#[from] nix_derivation::store_path::Error),

    #[error(transparent)]
    NarInfoBuildError(#[from] nix_narinfo::BuildError),

    #[error(transparent)]
    NarInfoParseError(#[from] nix_narinfo::ParseError),

    #[error(transparent)]
    IoError(#[from] std::io::Error),

    #[error(transparent)]
    JsonError(#[from] serde_json::Error),

    #[error(transparent)]
    LExprError(#[from] serde_lexpr::Error),

    #[error(transparent)]
    ParsingError(#[from] crate::model::ParsingError),
}

#[derive(Debug)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn new(url: &Url) -> Result<Self, StoreError> {
        if url.scheme() != "file" {
            return Err(StoreError::UsupportedSchema);
        }
        Ok(Self {
            root: match (
                match url.host_str() {
                    None | Some("localhost") => "",
                    Some(host) => host,
                },
                url.path(),
            ) {
                ("", "") => PathBuf::from("."),
                ("", path) => PathBuf::from(path),
                (host, "") => PathBuf::from(host),
                (host, path) => PathBuf::from(format!("{host}{path}")),
            },
        })
    }

    pub fn init(&self) -> Result<(), StoreError> {
        fs::create_dir_all(self.root.join("nar"))?;
        fs::write(
            self.root.join("nix-cache-info"),
            format!("StoreDir: {}\n", StoreDir::default().as_str()),
        )?;

        Ok(())
    }

    pub fn insert_package(
        &self,
        meta: &ComposerPackageMetadata,
        data: &Tree,
    ) -> Result<StorePath, StoreError> {
        let nar = data.into_nar()?;
        let nar_hash = hash_bytes(HashAlgorithm::Sha256, &nar);
        let nar_size = nar.len() as u64;

        let name = format!(
            "{}-{}-{}",
            meta.name.vendor(),
            meta.name.project(),
            meta.version,
        );

        let store_path = build_ca_path(
            &name,
            &CAHash::Nar(nar_hash.clone()),
            std::iter::empty(),
            false,
        )?;

        let nar_basename = format!("nar/{}.nar", nixbase32::encode(nar_hash.digest_as_bytes()));
        let narinfo_basename = format!("{}.narinfo", nixbase32::encode(store_path.digest()));

        let narinfo = NarInfo::builder(
            store_path.clone(),
            nar_basename.clone(),
            nar_hash.clone(),
            nar_size,
        )
        .compression(Compression::None)
        .content_address(Some(CAHash::Nar(nar_hash.clone())))
        .file_hash(Some(nar_hash))
        .file_size(Some(nar_size))
        .extension(Field {
            name: "X-Nixposer-Type".into(),
            value: "Package".into(),
        })
        .extension(Field {
            name: "X-Nixposer-Name".into(),
            value: meta.name.to_string(),
        })
        .extension(Field {
            name: "X-Nixposer-Version".into(),
            value: meta.version.to_string(),
        })
        .extension(Field {
            name: "X-Nixposer-Dependencies".into(),
            value: serde_lexpr::to_string(&meta.dependencies)?,
        })
        .extension(Field {
            name: "X-Nixposer-Autoload".into(),
            value: serde_lexpr::to_string(&meta.autoload)?,
        })
        .build()?;

        fs::write(self.root.join(&nar_basename), &nar)?;
        fs::write(
            self.root.join(&narinfo_basename),
            narinfo.to_canonical_bytes(),
        )?;

        Ok(store_path)
    }

    pub fn insert_repository(&self, data: &Tree) -> Result<StorePath, StoreError> {
        let nar = data.into_nar()?;
        let nar_hash = hash_bytes(HashAlgorithm::Sha256, &nar);
        let nar_size = nar.len() as u64;

        let name = if data.is_dir() {
            "nixposer-repository"
        } else {
            "nixposer-packages.json"
        }
        .to_string();

        let store_path = build_ca_path(
            &name,
            &CAHash::Nar(nar_hash.clone()),
            std::iter::empty(),
            false,
        )?;

        let nar_basename = format!("nar/{}.nar", nixbase32::encode(nar_hash.digest_as_bytes()));
        let narinfo_basename = format!("{}.narinfo", nixbase32::encode(store_path.digest()));

        let narinfo = NarInfo::builder(
            store_path.clone(),
            nar_basename.clone(),
            nar_hash.clone(),
            nar_size,
        )
        .compression(Compression::None)
        .content_address(Some(CAHash::Nar(nar_hash.clone())))
        .file_hash(Some(nar_hash))
        .file_size(Some(nar_size))
        .extension(Field {
            name: "X-Nixposer-Type".into(),
            value: "Repository".into(),
        })
        .build()?;

        fs::write(self.root.join(&nar_basename), &nar)?;
        fs::write(
            self.root.join(&narinfo_basename),
            narinfo.to_canonical_bytes(),
        )?;

        Ok(store_path)
    }

    pub fn index(&self) -> Result<Vec<(ComposerPackageMetadata, StorePath)>, StoreError> {
        Ok(self
            .root
            .read_dir()?
            .map(|entry| -> Result<Option<NarInfo>, StoreError> {
                let path = entry?.path();

                if path.extension().is_none_or(|ext| ext != "narinfo") {
                    return Ok(None);
                }

                let bytes = fs::read(&path)?;
                let narinfo = NarInfo::parse_in(&StoreDir::default(), &bytes)?;

                Ok(Some(narinfo))
            })
            .filter_map(Result::transpose)
            .collect::<Result<Vec<NarInfo>, StoreError>>()?
            .iter()
            .filter(|nar_info| {
                nar_info.extensions().contains(&Field {
                    name: "X-Nixposer-Type".into(),
                    value: "Package".into(),
                })
            })
            .map(
                |nar_info| -> Result<(ComposerPackageMetadata, StorePath), StoreError> {
                    let extextensions: HashMap<_, _> = nar_info
                        .extensions()
                        .iter()
                        .map(|f| (f.name.clone(), f.value.clone()))
                        .collect();

                    Ok((
                        ComposerPackageMetadata {
                            name: extextensions
                                .get("X-Nixposer-Name")
                                .ok_or(StoreError::InvalideNarinfoExteions)?
                                .parse()?,
                            version: extextensions
                                .get("X-Nixposer-Version")
                                .ok_or(StoreError::InvalideNarinfoExteions)?
                                .parse()?,
                            dependencies: extextensions
                                .get("X-Nixposer-Dependencies")
                                .map(|v| serde_lexpr::from_str(v.as_str()))
                                .ok_or(StoreError::InvalideNarinfoExteions)??,
                            autoload: extextensions
                                .get("X-Nixposer-Autoload")
                                .map(|v| serde_lexpr::from_str(v.as_str()))
                                .ok_or(StoreError::InvalideNarinfoExteions)??,
                        },
                        nar_info.store_path().clone(),
                    ))
                },
            )
            .collect::<Result<Vec<(ComposerPackageMetadata, StorePath)>, StoreError>>()?
            .into_iter()
            .sorted_by_key(|(meta, _)| (meta.name.clone(), meta.version.clone()))
            .collect())
    }
}
